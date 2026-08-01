//! robots.txt per-domain cache：读写分离 + single-flight + negative TTL。

use super::parser::{parse_robots_text, RobotsRules};
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use wisp_http::Client;

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
    /// 创建新的 Robots 缓存。
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
    fn domain_for(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let host = parsed.host_str()?;
        Some(match parsed.port() {
            Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
            None => format!("{}://{}", parsed.scheme(), host),
        })
    }

    async fn fetch_and_cache_rules(&self, client: &Client, domain: &str) -> RobotsRules {
        match self.fetch_robots(client, domain).await {
            Some(rules) => {
                self.cache
                    .insert(domain.into(), CacheEntry::Rules(rules.clone()));
                rules
            }
            None => {
                self.cache.insert(
                    domain.into(),
                    CacheEntry::Failed {
                        expires_at: Instant::now() + self.negative_ttl,
                    },
                );
                RobotsRules::default()
            }
        }
    }

    /// 三阶段：
    /// 1. 无锁读 cache（hit → 直接返回，零 I/O）
    /// 2. cache miss / Failed 过期 → per-domain single-flight 锁
    /// 3. double-check → fetch → 写缓存（Rules 或 Failed+TTL）
    pub async fn rules_for(&self, client: &Client, url: &str) -> RobotsRules {
        let Some(domain) = Self::domain_for(url) else {
            return RobotsRules::default();
        };
        if let Some(rules) = self.check_cache(&domain) {
            return rules;
        }
        let lock = self
            .loading
            .entry(domain.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;
        if let Some(rules) = self.check_cache(&domain) {
            return rules;
        }
        self.fetch_and_cache_rules(client, &domain).await
    }

    /// fetch robots.txt。成功返回 `Some(rules)`（可能空规则），失败返回 `None`。
    async fn fetch_robots(&self, client: &Client, domain: &str) -> Option<RobotsRules> {
        let robots_url = format!("{}/robots.txt", domain);
        let resp = client.get(&robots_url, &[]).await.ok()?;
        let text = resp.text().ok()?;
        Some(parse_robots_text(&text))
    }
}
