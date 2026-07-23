//! robots.txt parsing and caching.

use std::collections::HashMap;
use crate::http::Client;

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
    /// 用于判断 fetch_robots 是否成功获取到有效规则，
    /// 区分"无规则"与"获取失败返回的默认空规则"——失败时返回的空规则不应被缓存。
    pub fn is_empty_rules(&self) -> bool {
        self.disallowed.is_empty() && self.crawl_delay.is_none() && self.request_rate.is_none()
    }
}

/// Cache of robots.txt rules per domain.
pub struct RobotsCache {
    cache: HashMap<String, RobotsRules>,
}

impl RobotsCache {
    pub fn new() -> Self { Self { cache: HashMap::new() } }

    /// Check if a URL is allowed by robots.txt.
    pub async fn is_allowed(&mut self, client: &Client, url: &str) -> bool {
        let rules = self.rules_for(client, url).await;
        let path = url::Url::parse(url)
            .map(|p| p.path().to_string())
            .unwrap_or_default();
        rules.disallowed.iter().all(|d| !path.starts_with(d))
    }

    /// 获取 URL 对应域名的 Crawl-delay（秒）
    pub async fn crawl_delay(&mut self, client: &Client, url: &str) -> Option<f64> {
        self.rules_for(client, url).await.crawl_delay
    }

    /// 获取 URL 对应域名的所有规则
    pub async fn rules_for(&mut self, client: &Client, url: &str) -> RobotsRules {
        let Ok(parsed) = url::Url::parse(url) else { return RobotsRules::default(); };
        let Some(host) = parsed.host_str() else { return RobotsRules::default(); };
        // 保留端口：http://example.com:8080 与 http://example.com 是不同 origin，
        // robots.txt 必须从对应 host:port 获取，否则非默认端口会错误地从 80/443 拉取。
        let domain = match parsed.port() {
            Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
            None => format!("{}://{}", parsed.scheme(), host),
        };

        if !self.cache.contains_key(&domain) {
            let rules = self.fetch_robots(client, &domain).await;
            // 仅在成功获取到有效规则时缓存；fetch 失败返回的空规则不缓存，
            // 避免网络瞬态失败后被永久缓存为"允许全部"。
            if !rules.is_empty_rules() {
                self.cache.insert(domain.clone(), rules);
            }
        }

        self.cache.get(&domain).cloned().unwrap_or_default()
    }

    async fn fetch_robots(&self, client: &Client, domain: &str) -> RobotsRules {
        let robots_url = format!("{}/robots.txt", domain);
        let Ok(resp) = client.get(&robots_url).await else { return RobotsRules::default(); };
        let Ok(text) = resp.text() else { return RobotsRules::default(); };
        parse_robots_text(&text)
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
        if line.is_empty() || line.starts_with('#') { continue; }
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
}
