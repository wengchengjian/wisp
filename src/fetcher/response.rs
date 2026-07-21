//! 缁熶竴鍝嶅簲鍜岃姹傜被鍨嬨€?
//!
//! 鎵€鏈?Fetcher 妯″紡锛圚ttp / Dynamic / Stealth锛夎繑鍥炲悓涓€涓?`Response`锛?
//! 鐢ㄦ埛鏃犻渶鍏冲績搴曞眰瀹炵幇鍗冲彲浣跨敤 `.css()` / `.xpath()` / `.json()` 绛?API銆?

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use serde_json::Value;

use crate::error::{WispError, Result};
use crate::parser::{Node, NodeList};

/// HTTP 鏂规硶銆?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

/// 缁熶竴璇锋眰绫诲瀷銆?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub url: String,
    pub method: Method,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    /// 鐢ㄦ埛鑷畾涔夊厓鏁版嵁锛圫pider 鍦烘櫙浼犻€掓繁搴︺€佸洖璋冪瓑锛?
    #[serde(skip)]
    pub meta: Value,
    /// Spider 鍥炶皟鍚嶇О
    pub callback: Option<String>,
    /// 浼樺厛绾э紙Spider 璋冨害鐢級
    pub priority: i32,
}

impl Request {
    /// 鍒涘缓 GET 璇锋眰銆?
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

    /// 鍒涘缓 POST 璇锋眰銆?
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

    /// 璁剧疆鑷畾涔?header銆?
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    /// 璁剧疆鍏冩暟鎹€?
    pub fn with_meta(mut self, meta: Value) -> Self {
        self.meta = meta;
        self
    }

    /// 璁剧疆浼樺厛绾с€?
    pub fn with_priority(mut self, p: i32) -> Self {
        self.priority = p;
        self
    }

    /// 璁剧疆鍥炶皟鍚嶇О銆?
    pub fn with_callback(mut self, cb: &str) -> Self {
        self.callback = Some(cb.to_string());
        self
    }
}

/// 缁熶竴鍝嶅簲绫诲瀷 - 鎵€鏈?Fetcher 妯″紡杩斿洖姝ょ被鍨嬨€?
///
/// # 绀轰緥
///
/// ```rust,no_run
/// use wisp::Fetcher;
///
/// # async fn example() -> wisp::Result<()> {
/// let page = Fetcher::http().get("https://quotes.toscrape.com/").await?;
///
/// // 缁熶竴鐨勮В鏋?API
/// let quotes = page.css(".quote .text");
/// let authors = page.xpath("//small[@class='author']");
/// let title = page.title();
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Response {
    /// HTTP 鐘舵€佺爜
    pub status: u16,
    /// 鏈€缁?URL锛堥噸瀹氬悜鍚庯級
    pub url: String,
    /// 鍝嶅簲澶?
    pub headers: HashMap<String, String>,
    /// 鍝嶅簲浣撳師濮嬪瓧鑺?
    pub body: Vec<u8>,
    /// 娴忚鍣ㄦā寮忎笅鐨勯〉闈㈡爣棰?
    pub title: Option<String>,
    /// 娴忚鍣ㄦā寮忎笅鐨?cookies
    pub cookies: Vec<String>,
    /// 鍙戣捣姝ゅ搷搴旂殑璇锋眰锛堢敤浜?follow()锛?
    pub request: Option<Request>,
    /// Content-Type 澶达紙鐢ㄤ簬缂栫爜妫€娴嬶級
    pub(crate) content_type: String,
}

impl Response {
    /// 浠?HTTP 鍝嶅簲鏋勫缓銆?
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

    /// 浠庢祻瑙堝櫒鍝嶅簲鏋勫缓銆?
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

    // === 鏂囨湰/鏁版嵁 ===

    /// 瑙ｇ爜鍝嶅簲浣撲负鏂囨湰锛堣嚜鍔ㄥ瓧绗﹂泦妫€娴嬶級銆?
    pub fn text(&self) -> Result<String> {
        Ok(crate::http::encoding::decode(&self.body, &self.content_type))
    }

    /// 瑙ｆ瀽鍝嶅簲浣撲负 JSON銆?
    pub fn json(&self) -> Result<Value> {
        let text = self.text()?;
        serde_json::from_str(&text)
            .map_err(|e| WispError::CdpError(format!("JSON parse: {e}")))
    }

    /// 鐘舵€佺爜鏄惁涓?2xx銆?
    pub fn is_ok(&self) -> bool {
        self.status >= 200 && self.status < 300
    }

    /// 鑾峰彇椤甸潰鏍囬锛堟祻瑙堝櫒妯″紡锛夈€?
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    // === 瑙ｆ瀽锛堟牳蹇冪粺涓€鐐癸級===

    /// 瑙ｆ瀽 HTML 涓烘枃妗ｈ妭鐐广€?
    pub fn parse(&self) -> Node {
        let text = self.text().unwrap_or_default();
        Node::from_html(&text)
    }

    /// CSS 閫夋嫨鍣ㄦ煡璇紙蹇嵎鏂瑰紡锛夈€?
    pub fn css(&self, selector: &str) -> NodeList {
        self.parse().select(selector)
    }

    /// XPath 鏌ヨ锛堝揩鎹锋柟寮忥級銆?
    pub fn xpath(&self, expr: &str) -> NodeList {
        self.parse().xpath(expr)
    }

    /// 鎸夋枃鏈唴瀹规煡鎵惧厓绱犮€?
    pub fn find_by_text(&self, text: &str, tag: Option<&str>, exact: bool) -> NodeList {
        self.parse().find_by_text(text, tag, exact)
    }

    /// CSS 閫夋嫨鍣ㄦ煡璇㈢涓€涓尮閰嶅厓绱犮€?
    pub fn select_one(&self, selector: &str) -> Option<Node> {
        self.parse().select_one(selector)
    }

    // === 瀵艰埅 ===

    /// 浠庡綋鍓嶅搷搴?URL 瑙ｆ瀽鐩稿閾炬帴锛屽垱寤?GET 璇锋眰銆?
    pub fn follow(&self, href: &str) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(Request::get(&absolute))
    }

    /// 鍒涘缓甯?callback 鐨勮窡闅忚姹傘€?
    pub fn follow_with(&self, href: &str, callback: &str) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(Request::get(&absolute).with_callback(callback))
    }

    /// 鍒涘缓甯?meta 鐨勮窡闅忚姹傘€?
    pub fn follow_meta(&self, href: &str, meta: Value) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(Request::get(&absolute).with_meta(meta))
    }
}

/// 灏?href 瑙ｆ瀽涓虹粷瀵?URL銆?
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
