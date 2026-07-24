//! robots.txt parsing and caching.
//!
//! 并发设计：**读写分离 + single-flight + negative cache with TTL**。
//!
//! - **读路径**：`DashMap` 无锁读，cache hit 时零 I/O 零锁。
//! - **写路径**：cache miss 时 per-domain `Mutex` single-flight，确保同域名并发
//!   只 fetch 一次 robots.txt，其他请求等待结果后走缓存。
//! - **negative caching**：
//!   - fetch 成功（含 404/空规则）→ 缓存 `Rules`，永久有效。
//!   - fetch 失败（网络错误）→ 缓存 `Failed` + TTL（默认 60s），TTL 内不重试，
//!     避免反检测场景下频繁重试 robots.txt 被识别为扫描行为。TTL 过期后重新 fetch。
//!
//! 旧实现用 `Mutex<HashMap>` 包裹整个 cache，`is_allowed` 在持锁状态下调用
//! `fetch_robots`（发 HTTP 请求），导致同域名并发请求被完全串行化。

use crate::http::Client;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// 单域名的 robots.txt 规则
#[derive(Debug, Clone, Default)]
pub struct RobotsRules {
    /// Disallowed paths
    pub disallowed: Vec<String>,
    /// Crawl-delay 秒数（若存在）
    pub crawl_delay: Option<f64>,
    /// Request-rate (requests per second, 若存在)
    pub request_rate: Option<f64>,
}

impl RobotsRules {
    /// 规则是否为空（disallowed 空 + 无 crawl_delay + 无 request_rate）。
    pub fn is_empty_rules(&self) -> bool {
        self.disallowed.is_empty() && self.crawl_delay.is_none() && self.request_rate.is_none()
    }
}

/// 缓存条目：fetch 成功的规则或 fetch 失败的 negative 标记。
#[derive(Clone)]
enum CacheEntry {
    /// fetch 成功（含空规则——robots.txt 无限制或 404）
    Rules(RobotsRules),
    /// fetch 失败（网络错误），TTL 过期后重新 fetch
    Failed { expires_at: Instant },
}

/// Cache of robots.txt rules per domain.
///
/// 读写分离：`cache` 存已 fetch 的结果（`Rules` 或 `Failed`），`loading` 提供
/// per-domain single-flight 锁。读路径（cache hit）无锁无 I/O；写路径（cache miss）
/// 同域名并发只 fetch 一次。fetch 失败缓存 `Failed` + TTL，避免频繁重试。
pub struct RobotsCache {
    /// 缓存条目（Rules 或 Failed+TTL）
    cache: DashMap<String, CacheEntry>,
    /// per-domain single-flight 锁，确保同域名并发只 fetch 一次
    loading: DashMap<String, Arc<Mutex<()>>>,
    /// fetch 失败后的 negative cache TTL
    negative_ttl: Duration,
}

impl RobotsCache {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
            loading: DashMap::new(),
            negative_ttl: Duration::from_secs(60),
        }
    }

    /// 用指定 negative cache TTL 构造（主要用于测试）。
    pub fn with_negative_ttl(negative_ttl: Duration) -> Self {
        Self {
            cache: DashMap::new(),
            loading: DashMap::new(),
            negative_ttl,
        }
    }

    /// Check if a URL is allowed by robots.txt.
    pub async fn is_allowed(&self, client: &Client, url: &str) -> bool {
        let rules = self.rules_for(client, url).await;
        let path = url::Url::parse(url)
            .map(|p| p.path().to_string())
            .unwrap_or_default();
        rules.disallowed.iter().all(|d| !path.starts_with(d))
    }

    /// 获取 URL 对应域名的 Crawl-delay（秒）
    pub async fn crawl_delay(&self, client: &Client, url: &str) -> Option<f64> {
        self.rules_for(client, url).await.crawl_delay
    }

    /// 检查缓存。返回 `Some(rules)` 表示缓存有效（Rules 或未过期的 Failed），
    /// 返回 `None` 表示 cache miss 或 Failed 已过期（需要重新 fetch）。
    fn check_cache(&self, domain: &str) -> Option<RobotsRules> {
        if let Some(entry) = self.cache.get(domain) {
            match entry.clone() {
                CacheEntry::Rules(rules) => Some(rules),
                CacheEntry::Failed { expires_at } => {
                    if Instant::now() < expires_at {
                        Some(RobotsRules::default())
                    } else {
                        None // 过期，需要重新 fetch
                    }
                }
            }
        } else {
            None
        }
    }

    /// 获取 URL 对应域名的所有规则。
    ///
    /// 三阶段：
    /// 1. 无锁读 cache（hit → 直接返回，零 I/O）
    /// 2. cache miss / Failed 过期 → per-domain single-flight 锁
    /// 3. double-check → fetch → 写缓存（Rules 或 Failed+TTL）
    pub async fn rules_for(&self, client: &Client, url: &str) -> RobotsRules {
        let Ok(parsed) = url::Url::parse(url) else {
            return RobotsRules::default();
        };
        let Some(host) = parsed.host_str() else {
            return RobotsRules::default();
        };
        // 保留端口：http://example.com:8080 与 http://example.com 是不同 origin，
        // robots.txt 必须从对应 host:port 获取，否则非默认端口会错误地从 80/443 拉取。
        let domain = match parsed.port() {
            Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
            None => format!("{}://{}", parsed.scheme(), host),
        };

        // 1. 无锁读（cache hit 极快路径，不跨 await 持有 Ref）
        if let Some(rules) = self.check_cache(&domain) {
            return rules;
        }

        // 2. cache miss / Failed 过期 → 获取 per-domain single-flight 锁
        let lock = self
            .loading
            .entry(domain.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // 3. double-check（其他请求可能已 fetch 完）
        if let Some(rules) = self.check_cache(&domain) {
            return rules;
        }

        // 4. fetch（持 per-domain 锁，不阻塞其他域名）
        match self.fetch_robots(client, &domain).await {
            Some(rules) => {
                // fetch 成功 → 缓存 Rules（即使空规则，避免下次重复 fetch）
                self.cache.insert(domain, CacheEntry::Rules(rules.clone()));
                rules
            }
            None => {
                // fetch 失败 → 缓存 Failed + TTL，避免频繁重试（反检测场景）
                self.cache.insert(
                    domain,
                    CacheEntry::Failed {
                        expires_at: Instant::now() + self.negative_ttl,
                    },
                );
                RobotsRules::default()
            }
        }
    }

    /// fetch robots.txt。成功返回 `Some(rules)`（可能空规则），失败返回 `None`。
    async fn fetch_robots(&self, client: &Client, domain: &str) -> Option<RobotsRules> {
        let robots_url = format!("{}/robots.txt", domain);
        let resp = client.get(&robots_url, &[]).await.ok()?;
        let text = resp.text().ok()?;
        Some(parse_robots_text(&text))
    }
}

/// 解析 robots.txt 文本为 `RobotsRules`。
///
/// 仅采集 `User-agent: *` 段下的指令，支持 RFC 9309 的 `Disallow`，以及
/// `Crawl-delay`（秒）和 `Request-rate`（`N/D` 格式，转换为每秒请求数 N/D）。
/// 非法数值被静默忽略。空行和以 `#` 开头的注释行被跳过。
pub fn parse_robots_text(text: &str) -> RobotsRules {
    let mut rules = RobotsRules::default();
    let mut in_our_section = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("User-agent:") {
            let agent = line["User-agent:".len()..].trim();
            in_our_section = agent == "*";
        } else if in_our_section {
            if let Some(path) = line.strip_prefix("Disallow:") {
                let path = path.trim();
                if !path.is_empty() {
                    rules.disallowed.push(path.to_string());
                }
            } else if let Some(val) = line.strip_prefix("Crawl-delay:") {
                if let Ok(delay) = val.trim().parse::<f64>() {
                    rules.crawl_delay = Some(delay);
                }
            } else if let Some(val) = line.strip_prefix("Request-rate:") {
                // Request-rate: 1/5 (1 request per 5 seconds)
                // 解析 "N/D" 格式，取 N/D 作为每秒请求数
                let val = val.trim();
                if let Some(slash_pos) = val.find('/') {
                    let n_str = &val[..slash_pos];
                    let rest = &val[slash_pos + 1..];
                    // D 部分可能后跟注释 (空格分隔)
                    let d_str = rest.split_whitespace().next().unwrap_or("1");
                    if let (Ok(n), Ok(d)) = (n_str.parse::<f64>(), d_str.parse::<f64>()) {
                        if n > 0.0 && d > 0.0 {
                            rules.request_rate = Some(n / d);
                        }
                    }
                }
            }
        }
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_robots_rules_default() {
        let r = RobotsRules::default();
        assert!(r.disallowed.is_empty());
        assert!(r.crawl_delay.is_none());
        assert!(r.request_rate.is_none());
    }

    #[test]
    fn test_parse_crawl_delay() {
        let text = "User-agent: *\nCrawl-delay: 5\nDisallow: /private";
        let rules = parse_robots_text(text);
        assert_eq!(rules.crawl_delay, Some(5.0));
        assert!(rules.disallowed.contains(&"/private".to_string()));
        assert!(rules.request_rate.is_none());
    }

    #[test]
    fn test_parse_crawl_delay_ignored_outside_wildcard_section() {
        // Crawl-delay 在非 * 的 User-agent 段内不应被采集
        let text = "User-agent: GoogleBot\nCrawl-delay: 10\n\nUser-agent: *\nDisallow: /admin";
        let rules = parse_robots_text(text);
        assert_eq!(rules.crawl_delay, None);
        assert!(rules.disallowed.contains(&"/admin".to_string()));
    }

    #[test]
    fn test_parse_request_rate() {
        // Request-rate: 1/5 → 0.2 requests per second
        let text = "User-agent: *\nRequest-rate: 1/5\nDisallow: /slow";
        let rules = parse_robots_text(text);
        assert_eq!(rules.request_rate, Some(0.2));
        assert!(rules.disallowed.contains(&"/slow".to_string()));
    }

    #[test]
    fn test_parse_request_rate_with_optional_comment() {
        // Request-rate: 2/10 (during peak) → 0.2 req/s
        let text = "User-agent: *\nRequest-rate: 2/10 (during peak)";
        let rules = parse_robots_text(text);
        assert_eq!(rules.request_rate, Some(0.2));
    }

    #[test]
    fn test_parse_ignores_comments_and_empty_lines() {
        let text = "# 注释行\n\nUser-agent: *\n# 中间注释\nDisallow: /secret\n";
        let rules = parse_robots_text(text);
        assert!(rules.disallowed.contains(&"/secret".to_string()));
    }

    #[test]
    fn test_parse_empty_disallow_ignored() {
        // Disallow: (空) 按 RFC 9309 表示允许全部，不应加入 disallowed
        let text = "User-agent: *\nDisallow:";
        let rules = parse_robots_text(text);
        assert!(rules.disallowed.is_empty());
    }

    #[test]
    fn test_parse_invalid_crawl_delay_ignored() {
        let text = "User-agent: *\nCrawl-delay: not-a-number";
        let rules = parse_robots_text(text);
        assert!(rules.crawl_delay.is_none());
    }

    #[test]
    fn test_parse_invalid_request_rate_ignored() {
        let text = "User-agent: *\nRequest-rate: abc";
        let rules = parse_robots_text(text);
        assert!(rules.request_rate.is_none());
    }

    #[test]
    fn test_parse_empty_text_returns_default() {
        let rules = parse_robots_text("");
        assert!(rules.disallowed.is_empty());
        assert!(rules.crawl_delay.is_none());
        assert!(rules.request_rate.is_none());
    }

    #[test]
    fn test_is_empty_rules() {
        // 默认规则视为空（fetch 失败的返回值）
        assert!(RobotsRules::default().is_empty_rules());

        // 仅有 disallowed 不算空
        let mut r = RobotsRules::default();
        r.disallowed.push("/x".to_string());
        assert!(!r.is_empty_rules());

        // 仅有 crawl_delay 不算空
        let mut r = RobotsRules::default();
        r.crawl_delay = Some(1.0);
        assert!(!r.is_empty_rules());

        // 仅有 request_rate 不算空
        let mut r = RobotsRules::default();
        r.request_rate = Some(0.5);
        assert!(!r.is_empty_rules());
    }

    #[test]
    fn test_domain_key_preserves_port() {
        // 验证 url::Url::port() 在非默认端口下返回 Some，
        // 用以锁定 rules_for 构造 domain key 时包含端口的依赖假设。
        let parsed = url::Url::parse("http://example.com:8080/x").unwrap();
        assert_eq!(parsed.port(), Some(8080));
        let parsed_default = url::Url::parse("http://example.com/x").unwrap();
        assert_eq!(parsed_default.port(), None);
    }

    #[tokio::test]
    async fn test_negative_cache_on_fetch_failure() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        use tokio::net::TcpListener;

        // 起一个 server，accept 后立即 drop socket（模拟连接重置，fetch 失败）
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let fetch_count = Arc::new(AtomicUsize::new(0));
        let fetch_count_clone = fetch_count.clone();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                fetch_count_clone.fetch_add(1, Ordering::SeqCst);
                drop(socket); // 模拟连接重置
            }
        });

        let client = Client::new().unwrap();
        // TTL 100ms，测试不用等太久
        let cache = RobotsCache::with_negative_ttl(Duration::from_millis(100));
        let url = format!("http://{}/page", addr);

        // 第一次：fetch 失败 → 缓存 Failed
        let rules1 = cache.rules_for(&client, &url).await;
        assert!(rules1.is_empty_rules());
        assert_eq!(fetch_count.load(Ordering::SeqCst), 1, "第一次应触发 fetch");

        // 第二次：TTL 内 → 不重试，直接返回 default
        let rules2 = cache.rules_for(&client, &url).await;
        assert!(rules2.is_empty_rules());
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            1,
            "TTL 内不应重试 fetch"
        );

        // 等 TTL 过期
        tokio::time::sleep(Duration::from_millis(150)).await;

        // 第三次：TTL 过期 → 重新 fetch
        let rules3 = cache.rules_for(&client, &url).await;
        assert!(rules3.is_empty_rules());
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            2,
            "TTL 过期后应重新 fetch"
        );
    }

    #[tokio::test]
    async fn test_successful_fetch_caches_rules() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        // 起一个 server，返回有效 robots.txt
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let fetch_count = Arc::new(AtomicUsize::new(0));
        let fetch_count_clone = fetch_count.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                fetch_count_clone.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = socket.read(&mut buf).await;
                    let body = "User-agent: *\nDisallow: /private\n";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(resp.as_bytes()).await;
                });
            }
        });

        let client = Client::new().unwrap();
        let cache = RobotsCache::with_negative_ttl(Duration::from_secs(60));
        let url = format!("http://{}/page", addr);

        // 第一次：fetch 成功 → 缓存 Rules
        let rules1 = cache.rules_for(&client, &url).await;
        assert!(rules1.disallowed.contains(&"/private".to_string()));
        assert_eq!(fetch_count.load(Ordering::SeqCst), 1, "第一次应触发 fetch");

        // 第二次：缓存命中 → 不重新 fetch
        let rules2 = cache.rules_for(&client, &url).await;
        assert!(rules2.disallowed.contains(&"/private".to_string()));
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            1,
            "缓存命中不应重新 fetch"
        );
    }
}
