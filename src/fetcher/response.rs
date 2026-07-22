//! 统一响应和请求类型。
//!
//! 所有 Fetcher 模式（Http / Dynamic / Stealth）返回同一个 `Response`，
//! 用户无需关心底层实现即可使用 `.css()` / `.xpath()` / `.json()` 等 API。

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use serde_json::Value;

use crate::error::{WispError, Result};
use crate::parser::{Node, NodeList};

/// HTTP 方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

/// 统一请求类型。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub url: String,
    pub method: Method,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    /// 用户自定义元数据（Spider 场景传递深度、回调等）
    #[serde(skip)]
    pub meta: Value,
    /// Spider 回调名称
    pub callback: Option<String>,
    /// 优先级（Spider 调度用）
    pub priority: i32,
}

impl Request {
    /// 创建 GET 请求。
    pub fn get(url: &str) -> Self {
        Self {
            url: url.to_string(),
            method: Method::Get,
            headers: HashMap::new(),
            body: None,
            meta: Value::Null,
            callback: None,
            priority: 0,
        }
    }

    /// 创建 POST 请求。
    pub fn post(url: &str, body: Option<String>) -> Self {
        Self {
            url: url.to_string(),
            method: Method::Post,
            headers: HashMap::new(),
            body,
            meta: Value::Null,
            callback: None,
            priority: 0,
        }
    }

    /// 设置自定义 header。
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    /// 设置元数据。
    pub fn with_meta(mut self, meta: Value) -> Self {
        self.meta = meta;
        self
    }

    /// 设置优先级。
    pub fn with_priority(mut self, p: i32) -> Self {
        self.priority = p;
        self
    }

    /// 设置回调名称。
    pub fn with_callback(mut self, cb: &str) -> Self {
        self.callback = Some(cb.to_string());
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
/// let authors = page.xpath("//small[@class='author']");
/// let title = page.title();
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
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
    pub request: Option<Request>,
    /// Content-Type 头（用于编码检测）
    pub(crate) content_type: String,
}

impl Response {
    /// 从 HTTP 响应构建。
    pub fn from_http(
        status: u16,
        url: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
        content_type: String,
        request: Option<Request>,
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
        }
    }

    /// 从浏览器响应构建。
    pub fn from_browser(
        status: u16,
        url: String,
        html: String,
        title: String,
        cookies: Vec<String>,
        request: Option<Request>,
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
        }
    }

    // === 文本/数据 ===

    /// 解码响应体为文本（自动字符集检测）。
    pub fn text(&self) -> Result<String> {
        Ok(crate::http::encoding::decode(&self.body, &self.content_type))
    }

    /// 解析响应体为 JSON。
    pub fn json(&self) -> Result<Value> {
        let text = self.text()?;
        serde_json::from_str(&text)
            .map_err(|e| WispError::CdpError(format!("JSON parse: {e}")))
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
    pub fn parse(&self) -> Node {
        let text = self.text().unwrap_or_default();
        Node::from_html(&text)
    }

    /// CSS 选择器查询（快捷方式）。
    pub fn css(&self, selector: &str) -> NodeList {
        self.parse().select(selector)
    }

    /// XPath 查询（快捷方式）。
    pub fn xpath(&self, expr: &str) -> NodeList {
        self.parse().xpath(expr)
    }

    /// 按文本内容查找元素。
    pub fn find_by_text(&self, text: &str, tag: Option<&str>, exact: bool) -> NodeList {
        self.parse().find_by_text(text, tag, exact)
    }

    /// CSS 选择器查询第一个匹配元素。
    pub fn select_one(&self, selector: &str) -> Option<Node> {
        self.parse().select_one(selector)
    }

    // === 导航 ===

    /// 从当前响应 URL 解析相对链接，创建 GET 请求。
    pub fn follow(&self, href: &str) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(Request::get(&absolute))
    }

    /// 创建带 callback 的跟随请求。
    pub fn follow_with(&self, href: &str, callback: &str) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(Request::get(&absolute).with_callback(callback))
    }

    /// 创建带 meta 的跟随请求。
    pub fn follow_meta(&self, href: &str, meta: Value) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(Request::get(&absolute).with_meta(meta))
    }
}

/// 将 href 解析为绝对 URL。
fn resolve_href(base: &str, href: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    let base_url = url::Url::parse(base).ok()?;
    base_url.join(href).ok().map(|u| u.to_string())
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
            Some(Request::get("https://example.com/page")),
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
    fn test_response_xpath() {
        let resp = make_response(r#"<ul><li>X</li><li>Y</li></ul>"#);
        let items = resp.xpath("//li");
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_response_select_one() {
        let resp = make_response(r#"<p id="main">Content</p>"#);
        let node = resp.select_one("#main").unwrap();
        assert_eq!(node.text(), "Content");
    }

    #[test]
    fn test_response_find_by_text() {
        let resp = make_response(r#"<div>Apple</div><div>Banana</div>"#);
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
            None,
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

        let err_resp = Response { status: 404, ..resp };
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
            None,
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
}
