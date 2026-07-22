//! SpiderBuilder: 闭包式 Spider 定义，无需手写实现 trait。
//!
//! # 示例
//!
//! ## 简单爬虫（单 default handler）
//!
//! ```rust,no_run
//! use wisp::crawl::SpiderBuilder;
//! use std::time::Duration;
//!
//! let spider = SpiderBuilder::new("quotes")
//!     .start_urls(vec!["https://quotes.toscrape.com/"])
//!     .delay(Duration::from_millis(500))
//!     .obey_robots(false)
//!     .on("default", |resp| async move {
//!         let doc = resp.parse().unwrap();
//!         let items = doc.select(".quote").iter().map(|q| {
//!             serde_json::json!({ "text": q.select_one(".text").map(|n| n.text()) })
//!         }).collect();
//!         (items, vec![])
//!     })
//!     .build();
//! ```
//!
//! ## 多 callback 路由（列表 → 详情 → 内容）
//!
//! ```rust,no_run
//! use wisp::crawl::SpiderBuilder;
//! use wisp::crawl::stop::MaxPages;
//!
//! let spider = SpiderBuilder::new("pipeline")
//!     .start_urls(vec!["https://example.com/list"])
//!     .on("default", |resp| async move {
//!         // 列表页：follow 到 "detail"
//!         let follows: Vec<_> = resp.css(".item a").iter()
//!             .filter_map(|a| resp.follow_with(a.attr("href").unwrap_or(""), "detail"))
//!             .collect();
//!         (vec![], follows)
//!     })
//!     .on("detail", |resp| async move {
//!         // 详情页：follow 到 "content"
//!         let follows: Vec<_> = resp.css("article a").iter()
//!             .filter_map(|a| resp.follow_with(a.attr("href").unwrap_or(""), "content"))
//!             .collect();
//!         (vec![], follows)
//!     })
//!     .on("content", |resp| async move {
//!         // 内容页：提取数据
//!         (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
//!     })
//!     .until(MaxPages(1000))
//!     .build();
//! ```

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use futures::future::BoxFuture;
use serde_json::Value;

use super::{Spider, SpiderRequest, SpiderResponse};
use crate::http;

/// 异步 handler 签名：接收 SpiderResponse，返回 (items, follows)。
///
/// 用 `Arc<dyn Fn(...) -> BoxFuture>` 让闭包可 Clone + 异步 + Send + Sync。
/// 每个 handler 捕获不同状态都满足同一签名。
pub type Handler = Arc<
    dyn Fn(SpiderResponse) -> BoxFuture<'static, (Vec<Value>, Vec<SpiderRequest>)>
        + Send + Sync
>;

/// 闭包式 Spider 构建器。
///
/// 允许通过链式调用 + 闭包定义 Spider，避免为简单爬虫手写 trait impl。
pub struct SpiderBuilder {
    name: String,
    start_urls: Vec<String>,
    handlers: HashMap<String, Handler>,
    allowed_domains: HashSet<String>,
    delay: Duration,
    obey_robots: bool,
    max_retries: u32,
    fetcher_config: http::Config,
    fetch_mode: crate::fetcher::FetchMode,
    auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
    auto_exclude: HashSet<String>,
    is_blocked_fn: Option<Box<dyn Fn(&SpiderResponse) -> bool + Send + Sync + 'static>>,
    until_cond: Arc<dyn super::stop::StopCondition>,
    middlewares: Vec<Arc<dyn super::middleware::Middleware>>,
    pipelines: Vec<Arc<dyn super::middleware::ItemPipeline>>,
}

impl SpiderBuilder {
    /// 创建新 SpiderBuilder（name 为必填）。
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            start_urls: Vec::new(),
            handlers: HashMap::new(),
            allowed_domains: HashSet::new(),
            delay: Duration::ZERO,
            obey_robots: true,
            max_retries: 3,
            fetcher_config: http::Config::default(),
            fetch_mode: crate::fetcher::FetchMode::Http,
            auto_rules: Vec::new(),
            auto_exclude: HashSet::new(),
            is_blocked_fn: None,
            until_cond: Arc::new(super::NeverStop),
            middlewares: Vec::new(),
            pipelines: Vec::new(),
        }
    }

    /// 设置起始 URL 列表。
    pub fn start_urls(mut self, urls: Vec<impl Into<String>>) -> Self {
        self.start_urls = urls.into_iter().map(|u| u.into()).collect();
        self
    }

    /// 设置允许的域名集合。
    pub fn allowed_domains(mut self, domains: Vec<impl Into<String>>) -> Self {
        self.allowed_domains = domains.into_iter().map(|d| d.into()).collect();
        self
    }

    /// 设置下载延迟。
    pub fn delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }

    /// 设置下载延迟（毫秒）。
    pub fn delay_ms(mut self, ms: u64) -> Self {
        self.delay = Duration::from_millis(ms);
        self
    }

    /// 是否遵守 robots.txt。
    pub fn obey_robots(mut self, obey: bool) -> Self {
        self.obey_robots = obey;
        self
    }

    /// 设置最大重试次数。
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// 设置 fetcher 配置。
    pub fn fetcher_config(mut self, config: http::Config) -> Self {
        self.fetcher_config = config;
        self
    }

    /// 设置抓取模式（Http / Dynamic / Stealth / Auto）。
    pub fn mode(mut self, mode: crate::fetcher::FetchMode) -> Self {
        self.fetch_mode = mode;
        self
    }

    /// Auto 模式：URL 正则规则（优先级最高）。
    ///
    /// 匹配该规则的 URL 直接使用指定模式，跳过 Auto 嗅探。
    pub fn auto_rule(mut self, pattern: &str, mode: crate::fetcher::FetchMode) -> Self {
        self.auto_rules.push((pattern.to_string(), mode));
        self
    }

    /// Auto 模式：可选选择器（返回 0 节点不触发升级）。
    pub fn auto_exclude(mut self, selectors: Vec<&str>) -> Self {
        for s in selectors {
            self.auto_exclude.insert(s.to_string());
        }
        self
    }

    /// 自定义阻塞检测逻辑。
    pub fn is_blocked<F>(mut self, f: F) -> Self
    where
        F: Fn(&SpiderResponse) -> bool + Send + Sync + 'static,
    {
        self.is_blocked_fn = Some(Box::new(f));
        self
    }

    /// 注册 handler。label 为 `"default"` 表示入口（无 callback 时调用）。
    ///
    /// 多 callback 路由：`resp.follow_with(url, "detail")` 产生的请求会被
    /// `on("detail", handler)` 注册的 handler 处理。
    ///
    /// 这是定义 Spider 解析逻辑的唯一 API：至少注册一个 handler（通常为
    /// `"default"`）才能 `build()`。
    pub fn on<F, Fut>(mut self, label: &str, handler: F) -> Self
    where
        F: Fn(SpiderResponse) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = (Vec<Value>, Vec<SpiderRequest>)> + Send + 'static,
    {
        let boxed: Handler = Arc::new(move |resp| Box::pin(handler(resp)));
        self.handlers.insert(label.to_string(), boxed);
        self
    }

    /// 预设：Sitemap 爬虫。
    ///
    /// 自动解析 sitemap.xml，提取 `<loc>` URL，follow 到指定 label 的 handler。
    ///
    /// # 示例
    /// ```ignore
    /// let spider = SpiderBuilder::sitemap("my_spider", vec!["https://x.com/sitemap.xml".into()], "content")
    ///     .on("content", |resp| async move {
    ///         (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
    ///     })
    ///     .build();
    /// ```
    pub fn sitemap(name: &str, sitemap_urls: Vec<String>, content_label: &str) -> Self {
        let label = content_label.to_string();
        SpiderBuilder::new(name)
            .start_urls(sitemap_urls)
            .on("default", move |resp| {
                let label = label.clone();
                async move {
                    let text = resp.text().unwrap_or_default();
                    let re = regex::Regex::new(r"<loc>\s*(.*?)\s*</loc>").unwrap();
                    let follows: Vec<SpiderRequest> = re
                        .captures_iter(&text)
                        .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
                        .filter(|u| !u.is_empty())
                        .map(|url| SpiderRequest::get(&url).with_callback(&label))
                        .collect();
                    (vec![], follows)
                }
            })
    }

    /// 设置终止条件策略。
    pub fn until<C: super::stop::StopCondition + 'static>(mut self, cond: C) -> Self {
        self.until_cond = Arc::new(cond);
        self
    }

    /// 添加请求/响应中间件。
    pub fn middleware<M: super::middleware::Middleware + 'static>(mut self, mw: M) -> Self {
        self.middlewares.push(Arc::new(mw));
        self
    }

    /// 添加 Item 管道。
    pub fn pipeline<P: super::middleware::ItemPipeline + 'static>(mut self, p: P) -> Self {
        self.pipelines.push(Arc::new(p));
        self
    }

    /// 构建 ClosureSpider 实例。
    ///
    /// # Panics
    /// 若未注册任何 handler（`on()` 未调用）则 panic。
    pub fn build(self) -> ClosureSpider {
        assert!(
            !self.handlers.is_empty(),
            "SpiderBuilder: 必须至少注册一个 handler（通过 on()）"
        );
        ClosureSpider {
            name: self.name,
            start_urls: self.start_urls,
            handlers: self.handlers,
            allowed_domains: self.allowed_domains,
            delay: self.delay,
            obey_robots: self.obey_robots,
            max_retries: self.max_retries,
            fetcher_config: self.fetcher_config,
            fetch_mode: self.fetch_mode,
            auto_rules: self.auto_rules,
            auto_exclude: self.auto_exclude,
            is_blocked_fn: self.is_blocked_fn,
            until_cond: self.until_cond,
            middlewares: self.middlewares,
            pipelines: self.pipelines,
        }
    }
}

/// 由 SpiderBuilder 构建的闭包式 Spider。
pub struct ClosureSpider {
    name: String,
    start_urls: Vec<String>,
    handlers: HashMap<String, Handler>,
    allowed_domains: HashSet<String>,
    delay: Duration,
    obey_robots: bool,
    max_retries: u32,
    fetcher_config: http::Config,
    fetch_mode: crate::fetcher::FetchMode,
    auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
    auto_exclude: HashSet<String>,
    is_blocked_fn: Option<Box<dyn Fn(&SpiderResponse) -> bool + Send + Sync + 'static>>,
    until_cond: Arc<dyn super::stop::StopCondition>,
    middlewares: Vec<Arc<dyn super::middleware::Middleware>>,
    pipelines: Vec<Arc<dyn super::middleware::ItemPipeline>>,
}

#[async_trait]
impl Spider for ClosureSpider {
    fn name(&self) -> &str { &self.name }
    fn start_urls(&self) -> Vec<String> { self.start_urls.clone() }
    fn allowed_domains(&self) -> HashSet<String> { self.allowed_domains.clone() }
    fn download_delay(&self) -> Duration { self.delay }
    fn obey_robots(&self) -> bool { self.obey_robots }
    fn max_retries(&self) -> u32 { self.max_retries }
    fn fetcher_config(&self) -> http::Config { self.fetcher_config.clone() }
    fn fetch_mode(&self) -> crate::fetcher::FetchMode { self.fetch_mode }
    fn auto_rules(&self) -> Vec<(String, crate::fetcher::FetchMode)> { self.auto_rules.clone() }
    fn auto_exclude(&self) -> HashSet<String> { self.auto_exclude.clone() }

    /// callback 路由：根据 `resp.request.callback` 查表分发。
    ///
    /// 路由顺序：
    /// 1. callback 为 `None` 或 `"default"` → "default" handler（若有）
    /// 2. callback 为其他 label → 对应 handler（若有）
    /// 3. label 无匹配 → 回退到 "default" handler
    /// 4. 都无 → 返回空
    async fn handle(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let label = resp.request.callback.as_deref().unwrap_or("default");
        match self.handlers.get(label) {
            Some(h) => h(resp).await,
            None => {
                // label 不匹配，回退到 "default" handler
                if let Some(default_h) = self.handlers.get("default") {
                    default_h(resp).await
                } else {
                    // 无 default handler，返回空
                    (vec![], vec![])
                }
            }
        }
    }

    /// parse 兜底：ClosureSpider 不再使用 parse 闭包，统一走 `handle()` 路由。
    /// 此实现仅满足 Spider trait 默认契约，返回空结果。
    async fn parse(&self, _response: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        (vec![], vec![])
    }

    fn is_blocked(&self, resp: &SpiderResponse) -> bool {
        if let Some(ref f) = self.is_blocked_fn {
            f(resp)
        } else {
            super::BLOCKED_STATUS_CODES.contains(&resp.status)
        }
    }

    fn until(&self) -> Arc<dyn super::stop::StopCondition> {
        Arc::clone(&self.until_cond)
    }

    fn middlewares(&self) -> Vec<Arc<dyn super::middleware::Middleware>> {
        self.middlewares.clone()
    }

    fn pipelines(&self) -> Vec<Arc<dyn super::middleware::ItemPipeline>> {
        self.pipelines.clone()
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
            .delay_ms(100)
            .obey_robots(false)
            .on("default", |_resp| async move {
                (vec![json!({"ok": true})], vec![])
            })
            .build();

        assert_eq!(spider.name(), "test");
        assert_eq!(spider.start_urls(), vec!["https://example.com/"]);
        assert_eq!(spider.download_delay(), Duration::from_millis(100));
        assert!(!spider.obey_robots());
    }

    #[test]
    fn test_spider_builder_allowed_domains() {
        let spider = SpiderBuilder::new("test")
            .start_urls(vec!["https://example.com/"])
            .allowed_domains(vec!["example.com"])
            .on("default", |_| async move { (vec![], vec![]) })
            .build();

        let domains = spider.allowed_domains();
        assert!(domains.contains("example.com"));
    }

    #[test]
    #[should_panic(expected = "必须至少注册一个 handler")]
    fn test_spider_builder_no_handler_panics() {
        let _spider = SpiderBuilder::new("test")
            .start_urls(vec!["https://example.com/"])
            .build();
    }

    #[tokio::test]
    async fn test_closure_spider_default_handler() {
        let spider = SpiderBuilder::new("test")
            .start_urls(vec!["https://example.com/"])
            .on("default", |resp| async move {
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
            from_cache: false,
        };

        let (items, follows) = spider.handle(resp).await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["title"], "Hello");
        assert!(follows.is_empty());
    }

    #[tokio::test]
    async fn test_closure_spider_async_handler() {
        let spider = SpiderBuilder::new("async-test")
            .start_urls(vec!["https://example.com/"])
            .on("default", |resp| async move {
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
            from_cache: false,
        };

        let (items, _) = spider.handle(resp).await;
        assert_eq!(items[0]["text"], "World");
    }

    #[test]
    fn test_closure_spider_custom_is_blocked() {
        let spider = SpiderBuilder::new("test")
            .start_urls(Vec::<String>::new())
            .on("default", |_| async move { (vec![], vec![]) })
            .is_blocked(|resp| resp.body.windows(7).any(|w| w == b"blocked"))
            .build();

        let resp = SpiderResponse {
            url: "http://x.com".into(),
            status: 200,
            headers: Default::default(),
            body: b"you are blocked".to_vec(),
            request: SpiderRequest::get("http://x.com"),
            tracker: None,
            from_cache: false,
        };
        assert!(spider.is_blocked(&resp));

        let ok_resp = SpiderResponse {
            body: b"welcome".to_vec(),
            ..resp
        };
        assert!(!spider.is_blocked(&ok_resp));
    }

    #[tokio::test]
    async fn test_closure_spider_handle_routes_by_callback() {
        // 验证 handle() 根据 callback label 路由分发
        let spider = SpiderBuilder::new("routing")
            .start_urls(vec!["https://example.com/"])
            .on("default", |_resp| async move {
                (vec![json!({"handler": "default"})], vec![])
            })
            .on("detail", |_resp| async move {
                (vec![json!({"handler": "detail"})], vec![])
            })
            .on("content", |resp| async move {
                let title = resp.css("h1").text().join("");
                (vec![json!({"handler": "content", "title": title})], vec![])
            })
            .build();

        // 1. callback=None → default handler
        let resp_default = SpiderResponse {
            url: "https://example.com/".into(),
            status: 200,
            headers: Default::default(),
            body: b"<html></html>".to_vec(),
            request: SpiderRequest::get("https://example.com/"),
            tracker: None,
            from_cache: false,
        };
        let (items, _) = spider.handle(resp_default).await;
        assert_eq!(items[0]["handler"], "default");

        // 2. callback="detail" → detail handler
        let resp_detail = SpiderResponse {
            url: "https://example.com/detail/1".into(),
            status: 200,
            headers: Default::default(),
            body: b"<html></html>".to_vec(),
            request: SpiderRequest::get("https://example.com/detail/1").with_callback("detail"),
            tracker: None,
            from_cache: false,
        };
        let (items, _) = spider.handle(resp_detail).await;
        assert_eq!(items[0]["handler"], "detail");

        // 3. callback="content" → content handler
        let resp_content = SpiderResponse {
            url: "https://example.com/content/1".into(),
            status: 200,
            headers: Default::default(),
            body: b"<html><h1>Title</h1></html>".to_vec(),
            request: SpiderRequest::get("https://example.com/content/1").with_callback("content"),
            tracker: None,
            from_cache: false,
        };
        let (items, _) = spider.handle(resp_content).await;
        assert_eq!(items[0]["handler"], "content");
        assert_eq!(items[0]["title"], "Title");

        // 4. callback="unknown" → 回退到 default handler
        let resp_unknown = SpiderResponse {
            url: "https://example.com/unknown".into(),
            status: 200,
            headers: Default::default(),
            body: b"<html></html>".to_vec(),
            request: SpiderRequest::get("https://example.com/unknown").with_callback("unknown"),
            tracker: None,
            from_cache: false,
        };
        let (items, _) = spider.handle(resp_unknown).await;
        assert_eq!(items[0]["handler"], "default");
    }

    #[tokio::test]
    async fn test_closure_spider_handle_default_handler() {
        // 无 callback 时，handle() 路由到 "default" handler
        let spider = SpiderBuilder::new("fallback")
            .start_urls(vec!["https://example.com/"])
            .on("default", |_resp| async move {
                (vec![json!({"via": "default"})], vec![])
            })
            .build();

        let resp = SpiderResponse {
            url: "https://example.com/".into(),
            status: 200,
            headers: Default::default(),
            body: b"<html></html>".to_vec(),
            request: SpiderRequest::get("https://example.com/"),
            tracker: None,
            from_cache: false,
        };
        let (items, _) = spider.handle(resp).await;
        assert_eq!(items[0]["via"], "default");
    }
}
