//! 统一响应和请求类型。
//!
//! 所有 Fetcher 模式（Http / Dynamic / Stealth）返回同一个 `Response`，
//! 用户无需关心底层实现即可使用 `.css()` / `.json()` 等 API。
//! Spider 引擎也复用同一套 Request/Response，避免类型重复。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use super::FetchMode;
use crate::error::{ParseError, Result, WispError};
use crate::parser::{Node, NodeList};
use crate::utils::resolve_href;

/// 自定义 serde：把 `serde_json::Value` 编码为 `Vec<u8>` JSON 字节，
/// 绕过 bincode 1.x 不支持 `deserialize_any` 的限制，使 meta 随 checkpoint 往返。
pub(crate) mod meta_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer>(v: &Value, s: S) -> Result<S::Ok, S::Error> {
        let bytes = serde_json::to_vec(v).map_err(serde::ser::Error::custom)?;
        bytes.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
        let bytes = Vec::<u8>::deserialize(d)?;
        serde_json::from_slice(&bytes).map_err(serde::de::Error::custom)
    }
}

/// HTTP 方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    /// GET 请求。
    Get,
    /// POST 请求。
    Post,
    /// PUT 请求。
    Put,
    /// DELETE 请求。
    Delete,
}

impl Method {
    /// 返回标准 HTTP 动词字符串（大写）。
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

/// 统一请求类型（Fetcher + Spider 共用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// 请求 URL。
    pub url: String,
    /// HTTP 方法。
    pub method: Method,
    /// 请求头。
    pub headers: HashMap<String, String>,
    /// 请求体（POST/PUT 用）。
    pub body: Option<String>,
    /// 用户自定义元数据（Spider 场景传递深度、回调等）
    #[serde(with = "meta_serde")]
    pub meta: Value,
    /// Spider 回调名称
    pub callback: Option<String>,
    /// 优先级（Spider 调度用）
    pub priority: i32,
    /// 深度：起始 URL 为 0，每 follow 一次 +1。
    #[serde(default)]
    pub depth: u32,
    /// 代理 URL（由 ProxyInjectionMiddleware 设置，引擎读取并应用）。
    #[serde(skip)]
    pub proxy: Option<String>,
    /// 抓取模式覆盖（由 StealthUpgradeMiddleware 等设置，引擎优先使用此模式）。
    #[serde(skip)]
    pub fetch_mode_override: Option<FetchMode>,
    /// 网络错误重试计数（由 engine 维护，中间件只读）。
    ///
    /// 与 `refetch_depth`（响应中间件 Refetch 计数，engine 局部变量）独立：
    /// - `retry_count`：fetch 失败后同步重试的次数，上限 `EngineConfig.max_retries`
    /// - `refetch_depth`：响应成功后业务重做的次数，上限 `EngineConfig.max_refetch_rounds`
    #[serde(skip)]
    pub retry_count: u32,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: Method::Get,
            headers: HashMap::new(),
            body: None,
            meta: Value::Null,
            callback: None,
            priority: 0,
            depth: 0,
            proxy: None,
            fetch_mode_override: None,
            retry_count: 0,
        }
    }
}

impl Request {
    /// 创建 GET 请求。
    #[must_use]
    pub fn get(url: &str) -> Self {
        Self {
            url: url.to_string(),
            method: Method::Get,
            headers: HashMap::new(),
            body: None,
            meta: Value::Null,
            callback: None,
            priority: 0,
            depth: 0,
            proxy: None,
            fetch_mode_override: None,
            retry_count: 0,
        }
    }

    /// 创建 POST 请求。
    #[must_use]
    pub fn post(url: &str, body: Option<String>) -> Self {
        Self {
            url: url.to_string(),
            method: Method::Post,
            headers: HashMap::new(),
            body,
            meta: Value::Null,
            callback: None,
            priority: 0,
            depth: 0,
            proxy: None,
            fetch_mode_override: None,
            retry_count: 0,
        }
    }

    /// 设置自定义 header。
    #[must_use]
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    /// 设置元数据。
    #[must_use]
    pub fn with_meta(mut self, meta: Value) -> Self {
        self.meta = meta;
        self
    }

    /// 设置优先级。
    #[must_use]
    pub fn with_priority(mut self, p: i32) -> Self {
        self.priority = p;
        self
    }

    /// 设置回调名称。
    #[must_use]
    pub fn with_callback(mut self, cb: &str) -> Self {
        self.callback = Some(cb.to_string());
        self
    }

    /// 设置深度。
    #[must_use]
    pub fn with_depth(mut self, d: u32) -> Self {
        self.depth = d;
        self
    }

    /// 设置代理 URL。
    #[must_use]
    pub fn with_proxy(mut self, proxy: &str) -> Self {
        self.proxy = Some(proxy.to_string());
        self
    }
}

/// 统一响应类型 - 所有 Fetcher 模式返回此类型。
///
/// # 示例
///
/// ```rust,no_run
/// use wisp::Fetcher;
///
/// # async fn example() -> wisp::Result<()> {
/// let page = Fetcher::http().get("https://quotes.toscrape.com/").await?;
///
/// // 统一的解析 API
/// let quotes = page.css(".quote .text");
/// let authors = page.css("small.author");
/// let title = page.title();
/// # Ok(())
/// # }
/// ```
pub struct Response {
    /// HTTP 状态码
    pub status: u16,
    /// 最终 URL（重定向后）
    pub url: String,
    /// 响应头
    pub headers: HashMap<String, String>,
    /// 响应体原始字节
    pub body: Vec<u8>,
    /// 浏览器模式下的页面标题
    pub title: Option<String>,
    /// 浏览器模式下的 cookies
    pub cookies: Vec<String>,
    /// 发起此响应的请求（用于 follow()）
    pub request: Request,
    /// Content-Type 头（用于编码检测）
    pub content_type: String,
    /// 是否来自缓存（缓存命中不算 pages_crawled）。
    #[doc(hidden)]
    pub from_cache: bool,
    /// 标记 HTML 是否已被解析过（每个 Response 只允许解析一次）。
    parsed: AtomicBool,
}

impl Clone for Response {
    fn clone(&self) -> Self {
        Self {
            status: self.status,
            url: self.url.clone(),
            headers: self.headers.clone(),
            body: self.body.clone(),
            title: self.title.clone(),
            cookies: self.cookies.clone(),
            request: self.request.clone(),
            content_type: self.content_type.clone(),
            from_cache: self.from_cache,
            // 克隆体获得全新的解析标记（未解析状态）
            parsed: AtomicBool::new(false),
        }
    }
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("url", &self.url)
            .field("headers", &self.headers)
            .field("body_len", &self.body.len())
            .field("title", &self.title)
            .field("cookies", &self.cookies)
            .field("request", &self.request)
            .field("content_type", &self.content_type)
            .field("from_cache", &self.from_cache)
            .field("parsed", &self.parsed.load(Ordering::Relaxed))
            .finish()
    }
}

impl Response {
    /// 从所有字段构建（内部使用，如 Engine 组装响应）。
    #[doc(hidden)]
    #[must_use]
    pub fn from_parts(
        status: u16,
        url: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
        title: Option<String>,
        cookies: Vec<String>,
        request: Request,
        content_type: String,
        from_cache: bool,
    ) -> Self {
        Self {
            status,
            url,
            headers,
            body,
            title,
            cookies,
            request,
            content_type,
            from_cache,
            parsed: AtomicBool::new(false),
        }
    }

    /// 从 HTTP 响应构建。
    #[must_use]
    pub fn from_http(
        status: u16,
        url: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
        content_type: String,
        request: Request,
    ) -> Self {
        Self {
            status,
            url,
            headers,
            body,
            title: None,
            cookies: Vec::new(),
            request,
            content_type,
            from_cache: false,
            parsed: AtomicBool::new(false),
        }
    }

    /// 从浏览器响应构建。
    #[must_use]
    pub fn from_browser(
        status: u16,
        url: String,
        html: String,
        title: String,
        cookies: Vec<String>,
        request: Request,
    ) -> Self {
        Self {
            status,
            url,
            headers: HashMap::new(),
            body: html.into_bytes(),
            title: Some(title),
            cookies,
            request,
            content_type: "text/html; charset=utf-8".to_string(),
            from_cache: false,
            parsed: AtomicBool::new(false),
        }
    }

    // === 文本/数据 ===

    /// 解码响应体为文本（自动字符集检测）。
    pub fn text(&self) -> Result<String> {
        Ok(crate::http::encoding::decode(
            &self.body,
            &self.content_type,
        ))
    }

    /// 解析响应体为 JSON。
    pub fn json(&self) -> Result<Value> {
        let text = self.text()?;
        serde_json::from_str(&text)
            .map_err(|e| WispError::Parse(ParseError::Json(format!("JSON parse: {e}"))))
    }

    /// 状态码是否为 2xx。
    pub fn is_ok(&self) -> bool {
        self.status >= 200 && self.status < 300
    }

    /// 获取页面标题（浏览器模式）。
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    // === 解析（核心统一点）===

    /// 解析 HTML 为文档节点。
    ///
    /// # Panics
    ///
    /// 每个 `Response` 实例只允许调用一次 `parse()`（含通过 `css()`/`select_one()`/
    /// `find_by_text()` 等便捷方法的间接调用）。重复调用将 panic。
    /// 若需多次查询，请在一次 `parse()` 返回的 `Node` 上操作：
    ///
    /// ```rust,no_run
    /// # use wisp::Response;
    /// # fn example(resp: &Response) {
    /// let doc = resp.parse();  // 解析一次
    /// let titles = doc.select("h1");
    /// let links = doc.select("a.link");
    /// # }
    /// ```
    pub fn parse(&self) -> Node {
        assert!(
            !self.parsed.swap(true, Ordering::Relaxed),
            "Response::parse() 已被调用过。每个 Response 只允许解析一次，\
             请在一次 parse() 返回的 Node 上执行多次查询，\
             而非多次调用 css()/select_one()/find_by_text()。"
        );
        let text = self.text().unwrap_or_default();
        Node::from_html(&text)
    }

    /// CSS 选择器查询（便捷方法，内部调用 `parse()`）。
    ///
    /// # Panics
    ///
    /// 若此 Response 已被解析过（含通过其他便捷方法），将 panic。
    pub fn css(&self, selector: &str) -> NodeList {
        self.parse().select(selector)
    }

    /// 按文本内容查找元素（便捷方法，内部调用 `parse()`）。
    ///
    /// # Panics
    ///
    /// 若此 Response 已被解析过（含通过其他便捷方法），将 panic。
    pub fn find_by_text(&self, text: &str, tag: Option<&str>, exact: bool) -> NodeList {
        self.parse().find_by_text(text, tag, exact)
    }

    /// CSS 选择器查询第一个匹配元素（便捷方法，内部调用 `parse()`）。
    ///
    /// # Panics
    ///
    /// 若此 Response 已被解析过（含通过其他便捷方法），将 panic。
    pub fn select_one(&self, selector: &str) -> Option<Node> {
        self.parse().select_one(selector)
    }

    // === 导航 ===

    /// 从当前响应 URL 解析相对链接，创建 GET 请求（depth 自动 +1）。
    pub fn follow(&self, href: &str) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(Request::get(&absolute).with_depth(self.request.depth + 1))
    }

    /// 创建带 callback 的跟随请求（depth 自动 +1）。
    pub fn follow_with(&self, href: &str, callback: &str) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(
            Request::get(&absolute)
                .with_callback(callback)
                .with_depth(self.request.depth + 1),
        )
    }

    /// 创建带 meta 的跟随请求（depth 自动 +1）。
    pub fn follow_meta(&self, href: &str, meta: Value) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(
            Request::get(&absolute)
                .with_meta(meta)
                .with_depth(self.request.depth + 1),
        )
    }

    /// 读取 meta 中的字符串字段。缺失/类型不符/meta 非 Object 时返回空字符串。
    ///
    /// 替代样板代码：`meta.get("x").and_then(|v| v.as_str()).unwrap_or("").to_string()`。
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use wisp::Response;
    /// # fn example(resp: &Response) {
    /// let title = resp.meta_str("title").to_string();  // 拥有所有权
    /// # }
    /// ```
    pub fn meta_str(&self, key: &str) -> &str {
        self.request
            .meta
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }

    /// 读取 meta 中的 u64 字段。缺失/类型不符/meta 非 Object 时返回 0。
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use wisp::Response;
    /// # fn example(resp: &Response) {
    /// let idx = resp.meta_u64("chapter_index") as usize;
    /// # }
    /// ```
    pub fn meta_u64(&self, key: &str) -> u64 {
        self.request
            .meta
            .get(key)
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    }

    /// 批量提取链接并构造带 callback 的 follow 请求。
    ///
    /// 按顺序尝试 `selectors`，第一个匹配到元素的 selector 即停止（多选择器回退）。
    /// 自动跳过 `href` 为空或文本为空白的 `<a>` 元素。
    ///
    /// 注意：即使该 selector 的所有匹配都被空值过滤掉，回退也会停止——
    /// 不会自动尝试下一个 selector。
    ///
    /// 等价于示例中的样板代码：
    /// ```ignore
    /// let selectors = [".txt-list li .s2 a", ".list2 ul li .name a", ...];
    /// for sel in &selectors {
    ///     let links = doc.select(sel);
    ///     if !links.is_empty() {
    ///         for a in links.iter() {
    ///             if let Some(href) = a.attr("href") {
    ///                 let text = a.text().trim().to_string();
    ///                 if !text.is_empty() && !href.is_empty() {
    ///                     if let Some(req) = resp.follow_with(&href, "detail") {
    ///                         follows.push(req);
    ///                     }
    ///                 }
    ///             }
    ///         }
    ///         break;
    ///     }
    /// }
    /// ```
    ///
    /// # Panics
    ///
    /// 内部调用 `self.parse()`，若此 Response 已被解析过（含通过 `css()`/`select_one()`
    /// 等便捷方法），将 panic。
    pub fn enqueue_links(&self, selectors: &[&str], callback: &str) -> Vec<Request> {
        self.enqueue_links_with(selectors, callback, |_| Some(Value::Null))
    }

    /// 批量提取链接 + 闭包注入 meta 的 follow 请求构造器。
    ///
    /// 闭包接收每个匹配的 `<a>` 节点，返回：
    /// - `Some(meta)`：构造带 meta 的 follow 请求；若 meta 为 `Value::Null` 则使用
    ///   默认 meta（不调用 `with_meta`）
    /// - `None`：跳过该链接
    ///
    /// 注意：即使该 selector 的所有匹配都被空值过滤掉，回退也会停止——
    /// 不会自动尝试下一个 selector。
    ///
    /// # Panics
    ///
    /// 同 `enqueue_links`：若 Response 已被解析过将 panic。
    pub fn enqueue_links_with<F>(
        &self,
        selectors: &[&str],
        callback: &str,
        meta_fn: F,
    ) -> Vec<Request>
    where
        F: Fn(&crate::parser::Node) -> Option<Value>,
    {
        let doc = self.parse();
        let mut follows = Vec::new();

        for sel in selectors {
            let links = doc.select(sel);
            if links.is_empty() {
                continue;
            }
            for a in links.iter() {
                let Some(href) = a.attr("href") else { continue };
                if href.is_empty() {
                    continue;
                }
                let text = a.text();
                if text.trim().is_empty() {
                    continue;
                }
                let Some(meta) = meta_fn(a) else { continue };
                let Some(req) = self.follow_with(&href, callback) else {
                    tracing::trace!("enqueue_links: 跳过无法解析的 href: {:?}", href);
                    continue;
                };
                follows.push(if meta.is_null() {
                    req
                } else {
                    req.with_meta(meta)
                });
            }
            // 多选择器回退：匹配到第一个非空 selector 即停止
            break;
        }

        follows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_response(html: &str) -> Response {
        Response::from_http(
            200,
            "https://example.com/page".to_string(),
            HashMap::new(),
            html.as_bytes().to_vec(),
            "text/html; charset=utf-8".to_string(),
            Request::get("https://example.com/page"),
        )
    }

    #[test]
    fn test_response_text() {
        let resp = make_response("<h1>Hello</h1>");
        assert_eq!(resp.text().unwrap(), "<h1>Hello</h1>");
    }

    #[test]
    fn test_response_css() {
        let resp = make_response(r#"<div class="item">A</div><div class="item">B</div>"#);
        let items = resp.css(".item");
        assert_eq!(items.len(), 2);
        assert_eq!(items.text(), vec!["A", "B"]);
    }

    #[test]
    fn test_response_select_one() {
        let resp = make_response(r#"<p id="main">Content</p>"#);
        let node = resp.select_one("#main").unwrap();
        assert_eq!(node.text(), "Content");
    }

    #[test]
    fn test_response_find_by_text() {
        let resp = make_response(r"<div>Apple</div><div>Banana</div>");
        let found = resp.find_by_text("Apple", Some("div"), true);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn test_response_json() {
        let resp = Response::from_http(
            200,
            "https://api.example.com/".to_string(),
            HashMap::new(),
            br#"{"key": "value"}"#.to_vec(),
            "application/json".to_string(),
            Request::get("https://api.example.com/"),
        );
        let json = resp.json().unwrap();
        assert_eq!(json["key"], "value");
    }

    #[test]
    fn test_response_follow_relative() {
        let resp = make_response("<a href='/next'>Next</a>");
        let req = resp.follow("/next").unwrap();
        assert_eq!(req.url, "https://example.com/next");
    }

    #[test]
    fn test_response_follow_absolute() {
        let resp = make_response("");
        let req = resp.follow("https://other.com/page").unwrap();
        assert_eq!(req.url, "https://other.com/page");
    }

    #[test]
    fn test_response_follow_with_callback() {
        let resp = make_response("");
        let req = resp.follow_with("/detail", "parse_detail").unwrap();
        assert_eq!(req.url, "https://example.com/detail");
        assert_eq!(req.callback, Some("parse_detail".to_string()));
    }

    #[test]
    fn test_response_is_ok() {
        let resp = make_response("");
        assert!(resp.is_ok());

        let err_resp = Response::from_http(
            404,
            "https://example.com/page".to_string(),
            HashMap::new(),
            Vec::new(),
            "text/html".to_string(),
            Request::get("https://example.com/page"),
        );
        assert!(!err_resp.is_ok());
    }

    #[test]
    fn test_response_title() {
        let resp = Response::from_browser(
            200,
            "https://example.com/".to_string(),
            "<html><body>Hi</body></html>".to_string(),
            "My Page".to_string(),
            vec!["sid=abc".to_string()],
            Request::get("https://example.com/"),
        );
        assert_eq!(resp.title(), Some("My Page"));
        assert_eq!(resp.cookies, vec!["sid=abc"]);
    }

    #[test]
    fn test_request_builder() {
        let req = Request::get("https://example.com/")
            .with_header("Accept", "text/html")
            .with_priority(5)
            .with_callback("parse_page")
            .with_meta(serde_json::json!({"depth": 1}));

        assert_eq!(req.method, Method::Get);
        assert_eq!(req.headers.get("Accept").unwrap(), "text/html");
        assert_eq!(req.priority, 5);
        assert_eq!(req.callback, Some("parse_page".to_string()));
        assert_eq!(req.meta["depth"], 1);
    }

    #[test]
    #[should_panic(expected = "Response::parse() 已被调用过")]
    fn test_response_parse_twice_panics() {
        let resp = make_response("<h1>Hello</h1><p>World</p>");
        let _doc = resp.parse();
        // 第二次解析应 panic
        let _doc2 = resp.parse();
    }

    #[test]
    #[should_panic(expected = "Response::parse() 已被调用过")]
    fn test_response_css_then_select_one_panics() {
        let resp = make_response(r#"<div class="a">A</div><p id="b">B</p>"#);
        let _items = resp.css(".a");
        // css() 已触发 parse，再调 select_one 应 panic
        let _node = resp.select_one("#b");
    }

    #[test]
    fn test_response_clone_resets_parsed_flag() {
        let resp = make_response("<h1>Title</h1>");
        let _doc = resp.parse();
        // 克隆体应可正常解析
        let cloned = resp.clone();
        let doc2 = cloned.parse();
        assert_eq!(doc2.select("h1").len(), 1);
    }

    #[test]
    fn test_response_meta_str_returns_value_when_present() {
        let mut req = Request::get("https://example.com/");
        req.meta = serde_json::json!({"title": "你好", "author": ""});
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            b"<html></html>".to_vec(),
            "text/html".into(),
            req,
        );
        assert_eq!(resp.meta_str("title"), "你好");
        assert_eq!(resp.meta_str("author"), "");
    }

    #[test]
    fn test_response_meta_str_returns_empty_when_missing() {
        let req = Request::get("https://example.com/");
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            b"<html></html>".to_vec(),
            "text/html".into(),
            req,
        );
        // meta 为 Null
        assert_eq!(resp.meta_str("title"), "");
    }

    #[test]
    fn test_response_meta_str_returns_empty_when_meta_not_object() {
        let mut req = Request::get("https://example.com/");
        req.meta = serde_json::Value::Null;
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            b"<html></html>".to_vec(),
            "text/html".into(),
            req,
        );
        assert_eq!(resp.meta_str("anything"), "");
    }

    #[test]
    fn test_response_meta_u64_returns_value_when_present() {
        let mut req = Request::get("https://example.com/");
        req.meta = serde_json::json!({"chapter_index": 42});
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            b"<html></html>".to_vec(),
            "text/html".into(),
            req,
        );
        assert_eq!(resp.meta_u64("chapter_index"), 42);
    }

    #[test]
    fn test_response_meta_u64_returns_zero_when_missing_or_invalid() {
        let mut req = Request::get("https://example.com/");
        req.meta = serde_json::json!({"chapter_index": "not a number"});
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            b"<html></html>".to_vec(),
            "text/html".into(),
            req,
        );
        assert_eq!(resp.meta_u64("chapter_index"), 0);
        assert_eq!(resp.meta_u64("missing"), 0);
    }

    #[test]
    fn test_enqueue_links_returns_follows_for_first_matching_selector() {
        let html = r#"<html><body>
        <ul class="txt-list"><li><span class="s2"><a href="/book/1">书1</a></span></li>
        <li><span class="s2"><a href="/book/2">书2</a></span></li>
    </ul></body></html>"#;
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            html.as_bytes().to_vec(),
            "text/html".into(),
            Request::get("https://example.com/"),
        );

        let follows = resp.enqueue_links(
            &[".txt-list li .s2 a", ".list2 ul li .name a"],
            "detail",
        );
        assert_eq!(follows.len(), 2);
        assert_eq!(follows[0].url, "https://example.com/book/1");
        assert_eq!(follows[0].callback.as_deref(), Some("detail"));
        assert_eq!(follows[1].url, "https://example.com/book/2");
        // 验证 meta == Null 时跳过 with_meta，使用 Request 默认 meta
        assert_eq!(follows[0].meta, serde_json::Value::Null);
    }

    #[test]
    fn test_enqueue_links_falls_back_to_next_selector_when_first_empty() {
        let html = r#"<html><body>
        <div class="list2"><ul><li><span class="name"><a href="/book/3">书3</a></span></li></ul></div>
    </body></html>"#;
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            html.as_bytes().to_vec(),
            "text/html".into(),
            Request::get("https://example.com/"),
        );

        let follows = resp.enqueue_links(
            &[".txt-list li .s2 a", ".list2 ul li .name a"],
            "detail",
        );
        assert_eq!(follows.len(), 1);
        assert_eq!(follows[0].url, "https://example.com/book/3");
    }

    #[test]
    fn test_enqueue_links_skips_empty_href_and_empty_text() {
        let html = r#"<html><body>
        <ul class="list"><li><a href="/book/1">书1</a></li>
        <li><a href="">空href</a></li>
        <li><a href="/book/2">   </a></li>
        <li><a href="/book/3">书3</a></li>
    </ul></body></html>"#;
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            html.as_bytes().to_vec(),
            "text/html".into(),
            Request::get("https://example.com/"),
        );

        let follows = resp.enqueue_links(&[".list a"], "detail");
        assert_eq!(follows.len(), 2);
        assert_eq!(follows[0].url, "https://example.com/book/1");
        assert_eq!(follows[1].url, "https://example.com/book/3");
    }

    #[test]
    fn test_enqueue_links_returns_empty_when_no_selector_matches() {
        let html = r#"<html><body><p>no links here</p></body></html>"#;
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            html.as_bytes().to_vec(),
            "text/html".into(),
            Request::get("https://example.com/"),
        );

        let follows = resp.enqueue_links(&[".nonexistent a"], "detail");
        assert!(follows.is_empty());
    }

    #[test]
    fn test_enqueue_links_with_injects_meta_from_closure() {
        let html = r#"<html><body>
        <ul><li><a href="/book/1">书1</a></li>
        <li><a href="/book/2">书2</a></li>
    </ul></body></html>"#;
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            html.as_bytes().to_vec(),
            "text/html".into(),
            Request::get("https://example.com/"),
        );

        let follows = resp.enqueue_links_with(&["ul a"], "detail", |a| {
            let title = a.text().trim().to_string();
            if title.is_empty() {
                None
            } else {
                Some(serde_json::json!({"title": title, "author": ""}))
            }
        });
        assert_eq!(follows.len(), 2);
        assert_eq!(follows[0].meta, serde_json::json!({"title": "书1", "author": ""}));
        assert_eq!(follows[0].callback.as_deref(), Some("detail"));
        assert_eq!(follows[1].meta, serde_json::json!({"title": "书2", "author": ""}));
    }

    #[test]
    fn test_enqueue_links_with_skips_when_closure_returns_none() {
        let html = r#"<html><body>
        <ul>
            <li><a href="/book/1">keep</a></li>
            <li><a href="/book/2">skip</a></li>
        </ul>
    </body></html>"#;
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            html.as_bytes().to_vec(),
            "text/html".into(),
            Request::get("https://example.com/"),
        );

        let follows = resp.enqueue_links_with(&["ul a"], "detail", |a| {
            if a.text().trim() == "skip" {
                None
            } else {
                Some(serde_json::json!({}))
            }
        });
        assert_eq!(follows.len(), 1);
        assert_eq!(follows[0].url, "https://example.com/book/1");
    }

    #[test]
    #[should_panic(expected = "Response::parse() 已被调用过")]
    fn test_enqueue_links_panics_if_parse_already_called() {
        let html = r#"<html><body><a href="/x">x</a></body></html>"#;
        let resp = Response::from_http(
            200,
            "https://example.com/".into(),
            std::collections::HashMap::new(),
            html.as_bytes().to_vec(),
            "text/html".into(),
            Request::get("https://example.com/"),
        );
        let _ = resp.parse(); // 先占用
        let _ = resp.enqueue_links(&["a"], "detail"); // 应 panic
    }
}
