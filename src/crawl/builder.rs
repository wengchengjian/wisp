//! SpiderBuilder: 闂寘寮?Spider 瀹氫箟锛屾棤闇€鎵嬪姩瀹炵幇 trait銆?
//!
//! # 绀轰緥
//!
//! ```rust,no_run
//! use wisp::crawl::SpiderBuilder;
//! use std::time::Duration;
//!
//! let spider = SpiderBuilder::new("quotes")
//!     .start_urls(vec!["https://quotes.toscrape.com/"])
//!     .concurrent(10)
//!     .delay(Duration::from_millis(500))
//!     .obey_robots(false)
//!     .parse(|resp| {
//!         let doc = resp.parse().unwrap();
//!         let items = doc.select(".quote").iter().map(|q| {
//!             serde_json::json!({ "text": q.select_one(".text").map(|n| n.text()) })
//!         }).collect();
//!         (items, vec![])
//!     })
//!     .build();
//! ```

use std::collections::HashSet;
use std::time::Duration;
use async_trait::async_trait;
use serde_json::Value;

use super::{Spider, SpiderRequest, SpiderResponse};
use crate::http;

/// 瑙ｆ瀽闂寘绫诲瀷锛氭帴鏀?SpiderResponse锛岃繑鍥?(items, follow_requests)銆?
pub type ParseFn = Box<dyn Fn(SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) + Send + Sync + 'static>;

/// 寮傛瑙ｆ瀽闂寘绫诲瀷銆?
pub type AsyncParseFn = Box<dyn Fn(SpiderResponse) -> std::pin::Pin<Box<dyn std::future::Future<Output = (Vec<Value>, Vec<SpiderRequest>)> + Send>> + Send + Sync + 'static>;

/// 闂寘寮?Spider 鏋勫缓鍣ㄣ€?
///
/// 鍏佽閫氳繃閾惧紡璋冪敤 + 闂寘瀹氫箟 Spider锛岄伩鍏嶄负绠€鍗曠埇铏墜鍐?trait impl銆?
pub struct SpiderBuilder {
    name: String,
    start_urls: Vec<String>,
    allowed_domains: HashSet<String>,
    concurrent: u32,
    delay: Duration,
    obey_robots: bool,
    max_retries: u32,
    fetcher_config: http::Config,
    fetch_mode: crate::fetcher::FetchMode,
    auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
    auto_exclude: HashSet<String>,
    parse_fn: Option<ParseFn>,
    async_parse_fn: Option<AsyncParseFn>,
    is_blocked_fn: Option<Box<dyn Fn(&SpiderResponse) -> bool + Send + Sync + 'static>>,
}

impl SpiderBuilder {
    /// 鍒涘缓鏂?SpiderBuilder锛坣ame 涓哄繀濉級銆?
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            start_urls: Vec::new(),
            allowed_domains: HashSet::new(),
            concurrent: 8,
            delay: Duration::ZERO,
            obey_robots: true,
            max_retries: 3,
            fetcher_config: http::Config::default(),
            fetch_mode: crate::fetcher::FetchMode::Http,
            auto_rules: Vec::new(),
            auto_exclude: HashSet::new(),
            parse_fn: None,
            async_parse_fn: None,
            is_blocked_fn: None,
        }
    }

    /// 璁剧疆璧峰 URL 鍒楄〃銆?
    pub fn start_urls(mut self, urls: Vec<impl Into<String>>) -> Self {
        self.start_urls = urls.into_iter().map(|u| u.into()).collect();
        self
    }

    /// 璁剧疆鍏佽鐨勫煙鍚嶉泦鍚堛€?
    pub fn allowed_domains(mut self, domains: Vec<impl Into<String>>) -> Self {
        self.allowed_domains = domains.into_iter().map(|d| d.into()).collect();
        self
    }

    /// 璁剧疆骞跺彂璇锋眰鏁般€?
    pub fn concurrent(mut self, n: u32) -> Self {
        self.concurrent = n;
        self
    }

    /// 璁剧疆涓嬭浇寤惰繜銆?
    pub fn delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }

    /// 璁剧疆涓嬭浇寤惰繜锛堟绉掞級銆?
    pub fn delay_ms(mut self, ms: u64) -> Self {
        self.delay = Duration::from_millis(ms);
        self
    }

    /// 鏄惁閬靛畧 robots.txt銆?
    pub fn obey_robots(mut self, obey: bool) -> Self {
        self.obey_robots = obey;
        self
    }

    /// 璁剧疆鏈€澶ч噸璇曟鏁般€?
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// 璁剧疆 fetcher 閰嶇疆銆?
    pub fn fetcher_config(mut self, config: http::Config) -> Self {
        self.fetcher_config = config;
        self
    }

    /// 璁剧疆鐖彇妯″紡锛圚ttp / Dynamic / Stealth / Auto锛夈€?
    pub fn mode(mut self, mode: crate::fetcher::FetchMode) -> Self {
        self.fetch_mode = mode;
        self
    }

    /// Auto 妯″紡锛歎RL 姝ｅ垯瑙勫垯锛堜紭鍏堢骇鏈€楂橈級銆?
    ///
    /// 鍖归厤璇ユ鍒欑殑 URL 鐩存帴浣跨敤鎸囧畾妯″紡锛岃烦杩?Auto 妫€娴嬨€?
    pub fn auto_rule(mut self, pattern: &str, mode: crate::fetcher::FetchMode) -> Self {
        self.auto_rules.push((pattern.to_string(), mode));
        self
    }

    /// Auto 妯″紡锛氬彲閫夐€夋嫨鍣紙杩斿洖 0 鑺傜偣涓嶈Е鍙戝崌绾э級銆?
    pub fn auto_exclude(mut self, selectors: Vec<&str>) -> Self {
        for s in selectors {
            self.auto_exclude.insert(s.to_string());
        }
        self
    }

    /// 璁剧疆鍚屾瑙ｆ瀽闂寘銆?
    pub fn parse<F>(mut self, f: F) -> Self
    where
        F: Fn(SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) + Send + Sync + 'static,
    {
        self.parse_fn = Some(Box::new(f));
        self
    }

    /// 璁剧疆寮傛瑙ｆ瀽闂寘銆?
    pub fn parse_async<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SpiderResponse) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = (Vec<Value>, Vec<SpiderRequest>)> + Send + 'static,
    {
        self.async_parse_fn = Some(Box::new(move |resp| Box::pin(f(resp))));
        self
    }

    /// 鑷畾涔夐樆濉炴娴嬮€昏緫銆?
    pub fn is_blocked<F>(mut self, f: F) -> Self
    where
        F: Fn(&SpiderResponse) -> bool + Send + Sync + 'static,
    {
        self.is_blocked_fn = Some(Box::new(f));
        self
    }

    /// 鏋勫缓 ClosureSpider 瀹炰緥銆?
    ///
    /// # Panics
    /// 鑻ユ湭璁剧疆 parse 鎴?parse_async 闂寘鍒?panic銆?
    pub fn build(self) -> ClosureSpider {
        assert!(
            self.parse_fn.is_some() || self.async_parse_fn.is_some(),
            "SpiderBuilder: 蹇呴』璁剧疆 parse() 鎴?parse_async() 闂寘"
        );
        ClosureSpider {
            name: self.name,
            start_urls: self.start_urls,
            allowed_domains: self.allowed_domains,
            concurrent: self.concurrent,
            delay: self.delay,
            obey_robots: self.obey_robots,
            max_retries: self.max_retries,
            fetcher_config: self.fetcher_config,
            fetch_mode: self.fetch_mode,
            auto_rules: self.auto_rules,
            auto_exclude: self.auto_exclude,
            parse_fn: self.parse_fn,
            async_parse_fn: self.async_parse_fn,
            is_blocked_fn: self.is_blocked_fn,
        }
    }
}

/// 鐢?SpiderBuilder 鏋勫缓鐨勯棴鍖呭紡 Spider銆?
pub struct ClosureSpider {
    name: String,
    start_urls: Vec<String>,
    allowed_domains: HashSet<String>,
    concurrent: u32,
    delay: Duration,
    obey_robots: bool,
    max_retries: u32,
    fetcher_config: http::Config,
    fetch_mode: crate::fetcher::FetchMode,
    auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
    auto_exclude: HashSet<String>,
    parse_fn: Option<ParseFn>,
    async_parse_fn: Option<AsyncParseFn>,
    is_blocked_fn: Option<Box<dyn Fn(&SpiderResponse) -> bool + Send + Sync + 'static>>,
}

#[async_trait]
impl Spider for ClosureSpider {
    fn name(&self) -> &str { &self.name }
    fn start_urls(&self) -> Vec<String> { self.start_urls.clone() }
    fn allowed_domains(&self) -> HashSet<String> { self.allowed_domains.clone() }
    fn concurrent_requests(&self) -> u32 { self.concurrent }
    fn download_delay(&self) -> Duration { self.delay }
    fn obey_robots(&self) -> bool { self.obey_robots }
    fn max_retries(&self) -> u32 { self.max_retries }
    fn fetcher_config(&self) -> http::Config { self.fetcher_config.clone() }
    fn fetch_mode(&self) -> crate::fetcher::FetchMode { self.fetch_mode }
    fn auto_rules(&self) -> Vec<(String, crate::fetcher::FetchMode)> { self.auto_rules.clone() }
    fn auto_exclude(&self) -> HashSet<String> { self.auto_exclude.clone() }

    async fn parse(&self, response: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        if let Some(ref f) = self.async_parse_fn {
            f(response).await
        } else if let Some(ref f) = self.parse_fn {
            f(response)
        } else {
            (vec![], vec![])
        }
    }

    fn is_blocked(&self, resp: &SpiderResponse) -> bool {
        if let Some(ref f) = self.is_blocked_fn {
            f(resp)
        } else {
            super::BLOCKED_STATUS_CODES.contains(&resp.status)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_spider_builder_basic() {
        let spider = SpiderBuilder::new("test")
            .start_urls(vec!["https://example.com/"])
            .concurrent(4)
            .delay_ms(100)
            .obey_robots(false)
            .parse(|resp| {
                let _ = resp;
                (vec![json!({"ok": true})], vec![])
            })
            .build();

        assert_eq!(spider.name(), "test");
        assert_eq!(spider.start_urls(), vec!["https://example.com/"]);
        assert_eq!(spider.concurrent_requests(), 4);
        assert_eq!(spider.download_delay(), Duration::from_millis(100));
        assert!(!spider.obey_robots());
    }

    #[test]
    fn test_spider_builder_allowed_domains() {
        let spider = SpiderBuilder::new("test")
            .start_urls(vec!["https://example.com/"])
            .allowed_domains(vec!["example.com"])
            .parse(|_| (vec![], vec![]))
            .build();

        let domains = spider.allowed_domains();
        assert!(domains.contains("example.com"));
    }

    #[test]
    #[should_panic(expected = "蹇呴』璁剧疆 parse()")]
    fn test_spider_builder_no_parse_panics() {
        let _spider = SpiderBuilder::new("test")
            .start_urls(vec!["https://example.com/"])
            .build();
    }

    #[tokio::test]
    async fn test_closure_spider_parse() {
        let spider = SpiderBuilder::new("test")
            .start_urls(vec!["https://example.com/"])
            .parse(|resp| {
                let doc = resp.parse().unwrap();
                let title = doc.select_one("h1").map(|n| n.text()).unwrap_or_default();
                (vec![json!({"title": title})], vec![])
            })
            .build();

        let resp = SpiderResponse {
            url: "https://example.com/".into(),
            status: 200,
            headers: Default::default(),
            body: b"<html><body><h1>Hello</h1></body></html>".to_vec(),
            request: SpiderRequest::get("https://example.com/"),
            tracker: None,
        };

        let (items, follows) = spider.parse(resp).await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["title"], "Hello");
        assert!(follows.is_empty());
    }

    #[tokio::test]
    async fn test_closure_spider_parse_async() {
        let spider = SpiderBuilder::new("async-test")
            .start_urls(vec!["https://example.com/"])
            .parse_async(|resp| async move {
                let doc = resp.parse().unwrap();
                let text = doc.select_one("p").map(|n| n.text()).unwrap_or_default();
                (vec![json!({"text": text})], vec![])
            })
            .build();

        let resp = SpiderResponse {
            url: "https://example.com/".into(),
            status: 200,
            headers: Default::default(),
            body: b"<html><body><p>World</p></body></html>".to_vec(),
            request: SpiderRequest::get("https://example.com/"),
            tracker: None,
        };

        let (items, _) = spider.parse(resp).await;
        assert_eq!(items[0]["text"], "World");
    }

    #[test]
    fn test_closure_spider_custom_is_blocked() {
        let spider = SpiderBuilder::new("test")
            .start_urls(Vec::<String>::new())
            .parse(|_| (vec![], vec![]))
            .is_blocked(|resp| resp.body.windows(7).any(|w| w == b"blocked"))
            .build();

        let resp = SpiderResponse {
            url: "http://x.com".into(),
            status: 200,
            headers: Default::default(),
            body: b"you are blocked".to_vec(),
            request: SpiderRequest::get("http://x.com"),
            tracker: None,
        };
        assert!(spider.is_blocked(&resp));

        let ok_resp = SpiderResponse {
            body: b"welcome".to_vec(),
            ..resp
        };
        assert!(!spider.is_blocked(&ok_resp));
    }
}
