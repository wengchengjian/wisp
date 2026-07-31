//! SpiderBuilder: 闭包式 Spider 定义，无需手写实现 trait。
//!
//! # 示例
//!
//! ## 简单爬虫（单 default handler）
//!
//! ```rust,no_run
//! use wisp_crawl::SpiderBuilder;
//! use wisp_parser::ResponseExt;
//!
//! let spider = SpiderBuilder::new("quotes")
//!     .start_urls(vec!["https://quotes.toscrape.com/"])
//!     .on("default", |resp| async move {
//!         let doc = resp.parse();
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
//! use wisp_crawl::SpiderBuilder;
//! use wisp_crawl::stop::MaxPages;
//! use wisp_parser::ResponseExt;
//!
//! let spider = SpiderBuilder::new("pipeline")
//!     .start_urls(vec!["https://example.com/list"])
//!     .on("default", |resp| async move {
//!         // 列表页：follow 到 "detail"
//!         let follows: Vec<_> = resp.css(".item a").iter()
//!             .filter_map(|a| resp.follow_with(&a.attr("href").unwrap_or_default(), "detail"))
//!             .collect();
//!         (vec![], follows)
//!     })
//!     .on("detail", |resp| async move {
//!         // 详情页：follow 到 "content"
//!         let follows: Vec<_> = resp.css("article a").iter()
//!             .filter_map(|a| resp.follow_with(&a.attr("href").unwrap_or_default(), "content"))
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

use async_trait::async_trait;
use futures::future::BoxFuture;
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, LazyLock};

use super::page::Page;
use super::{Request, Response, Spider};
use wisp_parser::Node;

/// Sitemap `<loc>` 提取正则：匹配 `<loc>URL</loc>` 中的 URL（允许前后空白）。
static RE_SITEMAP_LOC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<loc>\s*(.*?)\s*</loc>").unwrap());

/// 异步 handler 签名：接收 Response，返回 (items, follows)。
///
/// 用 `Arc<dyn Fn(...) -> BoxFuture>` 让闭包可 Clone + 异步 + Send + Sync。
/// 每个 handler 捕获不同状态都满足同一签名。
pub type Handler =
    Arc<dyn Fn(Response) -> BoxFuture<'static, (Vec<Value>, Vec<Request>)> + Send + Sync>;

/// 闭包式 Spider 构建器。
///
/// 允许通过链式调用 + 闭包定义 Spider，避免为简单爬虫手写 trait impl。
///
/// ND-031-ARCH 修复：引擎配置（fetch_mode/obey_robots/max_retries/download_delay/auto_rules）
/// 已从 SpiderBuilder 移除，改用 `EngineBuilder` 配置。SpiderBuilder 只保留业务逻辑配置。
pub struct SpiderBuilder {
    name: String,
    start_urls: Vec<String>,
    handlers: HashMap<String, Handler>,
    allowed_domains: HashSet<String>,
    is_blocked_fn: Option<Box<dyn Fn(&Response) -> bool + Send + Sync + 'static>>,
    until_cond: Arc<dyn super::stop::StopCondition>,
}

impl SpiderBuilder {
    /// 创建新 SpiderBuilder（name 为必填）。
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            start_urls: Vec::new(),
            handlers: HashMap::new(),
            allowed_domains: HashSet::new(),
            is_blocked_fn: None,
            until_cond: Arc::new(super::NeverStop),
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

    /// 自定义阻塞检测逻辑。
    pub fn is_blocked<F>(mut self, f: F) -> Self
    where
        F: Fn(&Response) -> bool + Send + Sync + 'static,
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
        F: Fn(Response) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = (Vec<Value>, Vec<Request>)> + Send + 'static,
    {
        let boxed: Handler = Arc::new(move |resp| Box::pin(handler(resp)));
        self.handlers.insert(label.to_string(), boxed);
        self
    }

    /// 注册页面 handler：接收 [`Page`]，内部自动解析一次并收集 items/follows。
    ///
    /// 与 [`Self::on`] 不同，handler 是同步的，返回 `Page` 而非
    /// `(items, follows)`；`build()` 时统一转换为引擎所需格式。
    ///
    /// # 示例
    /// ```rust,no_run
    /// use wisp_crawl::SpiderBuilder;
    ///
    /// let spider = SpiderBuilder::new("quotes")
    ///     .start_urls(vec!["https://quotes.toscrape.com/"])
    ///     .on_page("default", |mut page| {
    ///         page.follow_links(&[".quote a"], "detail", |_page, _idx, a| {
    ///             serde_json::json!({ "title": a.text().trim() })
    ///         });
    ///         page
    ///     })
    ///     .on_page("detail", |mut page| {
    ///         page.item(serde_json::json!({ "title": page.meta_str("title") }));
    ///         page
    ///     })
    ///     .build();
    /// ```
    pub fn on_page<F>(mut self, label: &str, handler: F) -> Self
    where
        F: Fn(Page) -> Page + Send + Sync + 'static,
    {
        let boxed: Handler = Arc::new(move |resp| {
            let page = Page::new(resp);
            let (items, follows) = handler(page).finish();
            Box::pin(async move { (items, follows) })
        });
        self.handlers.insert(label.to_string(), boxed);
        self
    }

    /// 注册纯链接提取 handler：匹配第一个非空选择器，follow 到 `callback`。
    ///
    /// 适合“列表页只负责发现链接”的流程；需要同时产出 item 或诊断时用
    /// [`Self::on_page`]。
    pub fn on_links<F>(self, label: &str, selectors: &[&str], callback: &str, meta_for: F) -> Self
    where
        F: Fn(&Page, usize, &Node) -> Value + Send + Sync + 'static,
    {
        self.on_links_n(label, selectors, callback, usize::MAX, meta_for)
    }

    /// 注册纯链接提取 handler，最多跟随前 `limit` 个链接。
    pub fn on_links_n<F>(
        self,
        label: &str,
        selectors: &[&str],
        callback: &str,
        limit: usize,
        meta_for: F,
    ) -> Self
    where
        F: Fn(&Page, usize, &Node) -> Value + Send + Sync + 'static,
    {
        let selectors: Vec<String> = selectors.iter().map(|s| s.to_string()).collect();
        let callback = callback.to_string();
        self.on_page(label, move |mut page| {
            let selectors: Vec<&str> = selectors.iter().map(String::as_str).collect();
            page.follow_links_n(&selectors, &callback, limit, &meta_for);
            page
        })
    }

    /// 注册内容页 handler：从页面提取正文，合并 meta 字段并产出 item。
    ///
    /// item 由请求 meta + `content` + `url` 组成，适合“章节页收集正文”的流程。
    /// `clean` 接收原始文本并返回清洗后的文本。
    pub fn on_content<F>(self, label: &str, selectors: &[&str], clean: F) -> Self
    where
        F: Fn(String) -> String + Send + Sync + 'static,
    {
        let selectors: Vec<String> = selectors.iter().map(|s| s.to_string()).collect();
        self.on_page(label, move |mut page| {
            let selectors: Vec<&str> = selectors.iter().map(String::as_str).collect();
            let text = (&clean)(page.content_text(&selectors));
            let mut item = match page.meta_owned() {
                Value::Object(map) => Value::Object(map),
                _ => Value::Object(serde_json::Map::new()),
            };
            if let Value::Object(ref mut map) = item {
                map.insert("content".to_string(), Value::String(text));
                map.insert("url".to_string(), Value::String(page.url().to_string()));
            }
            page.item_value(item);
            page
        })
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
                    let follows: Vec<Request> = RE_SITEMAP_LOC
                        .captures_iter(&text)
                        .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
                        .filter(|u| !u.is_empty())
                        .map(|url| Request::get(&url).with_callback(&label))
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
            is_blocked_fn: self.is_blocked_fn,
            until_cond: self.until_cond,
        }
    }
}

/// 由 SpiderBuilder 构建的闭包式 Spider。
///
/// ND-031-ARCH：引擎配置字段已移除，ClosureSpider 只持有业务逻辑字段。
pub struct ClosureSpider {
    name: String,
    start_urls: Vec<String>,
    handlers: HashMap<String, Handler>,
    allowed_domains: HashSet<String>,
    is_blocked_fn: Option<Box<dyn Fn(&Response) -> bool + Send + Sync + 'static>>,
    until_cond: Arc<dyn super::stop::StopCondition>,
}

#[async_trait]
impl Spider for ClosureSpider {
    fn name(&self) -> &str {
        &self.name
    }
    fn start_urls(&self) -> Vec<String> {
        self.start_urls.clone()
    }
    fn allowed_domains(&self) -> HashSet<String> {
        self.allowed_domains.clone()
    }

    /// callback 路由：根据 `resp.request.callback` 查表分发。
    ///
    /// 路由顺序：
    /// 1. callback 为 `None` 或 `"default"` → "default" handler（若有）
    /// 2. callback 为其他 label → 对应 handler（若有）
    /// 3. label 无匹配 → 回退到 "default" handler
    /// 4. 都无 → 返回空
    async fn handle(&self, resp: Response) -> (Vec<Value>, Vec<Request>) {
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

    fn is_blocked(&self, resp: &Response) -> bool {
        if let Some(ref f) = self.is_blocked_fn {
            f(resp)
        } else {
            super::BLOCKED_STATUS_CODES.contains(&resp.status)
        }
    }

    fn until(&self) -> Arc<dyn super::stop::StopCondition> {
        Arc::clone(&self.until_cond)
    }

    fn accepts_callback(&self, callback: Option<&str>) -> bool {
        match callback {
            None => self.handlers.contains_key("default"),
            Some(cb) => self.handlers.contains_key(cb),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_parser::ResponseExt;
    use serde_json::json;

    #[test]
    fn test_spider_builder_basic() {
        let spider = SpiderBuilder::new("test")
            .start_urls(vec!["https://example.com/"])
            .on("default", |_resp| async move {
                (vec![json!({"ok": true})], vec![])
            })
            .build();

        assert_eq!(spider.name(), "test");
        assert_eq!(spider.start_urls(), vec!["https://example.com/"]);
        // ND-031-ARCH：download_delay/obey_robots 已迁移到 EngineBuilder
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
                let doc = resp.parse();
                let title = doc.select_one("h1").map(|n| n.text()).unwrap_or_default();
                (vec![json!({"title": title})], vec![])
            })
            .build();

        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            Default::default(),
            b"<html><body><h1>Hello</h1></body></html>".to_vec(),
            String::new(),
            Request::get("https://example.com/"),
        );

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
                let doc = resp.parse();
                let text = doc.select_one("p").map(|n| n.text()).unwrap_or_default();
                (vec![json!({"text": text})], vec![])
            })
            .build();

        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            Default::default(),
            b"<html><body><p>World</p></body></html>".to_vec(),
            String::new(),
            Request::get("https://example.com/"),
        );

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

        let resp = Response::from_http(
            200,
            "http://x.com".into(),
            Default::default(),
            b"you are blocked".to_vec(),
            String::new(),
            Request::get("http://x.com"),
        );
        assert!(spider.is_blocked(&resp));

        let ok_resp = Response::from_http(
            200,
            "http://x.com".into(),
            Default::default(),
            b"welcome".to_vec(),
            String::new(),
            Request::get("http://x.com"),
        );
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
        let resp_default = Response::from_http(
            200,
            "https://example.com/".into(),
            Default::default(),
            b"<html></html>".to_vec(),
            String::new(),
            Request::get("https://example.com/"),
        );
        let (items, _) = spider.handle(resp_default).await;
        assert_eq!(items[0]["handler"], "default");

        // 2. callback="detail" → detail handler
        let resp_detail = Response::from_http(
            200,
            "https://example.com/detail/1".into(),
            Default::default(),
            b"<html></html>".to_vec(),
            String::new(),
            Request::get("https://example.com/detail/1").with_callback("detail"),
        );
        let (items, _) = spider.handle(resp_detail).await;
        assert_eq!(items[0]["handler"], "detail");

        // 3. callback="content" → content handler
        let resp_content = Response::from_http(
            200,
            "https://example.com/content/1".into(),
            Default::default(),
            b"<html><h1>Title</h1></html>".to_vec(),
            String::new(),
            Request::get("https://example.com/content/1").with_callback("content"),
        );
        let (items, _) = spider.handle(resp_content).await;
        assert_eq!(items[0]["handler"], "content");
        assert_eq!(items[0]["title"], "Title");

        // 4. callback="unknown" → 回退到 default handler
        let resp_unknown = Response::from_http(
            200,
            "https://example.com/unknown".into(),
            Default::default(),
            b"<html></html>".to_vec(),
            String::new(),
            Request::get("https://example.com/unknown").with_callback("unknown"),
        );
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

        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            Default::default(),
            b"<html></html>".to_vec(),
            String::new(),
            Request::get("https://example.com/"),
        );
        let (items, _) = spider.handle(resp).await;
        assert_eq!(items[0]["via"], "default");
    }
}
