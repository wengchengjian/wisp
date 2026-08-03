//! 统一响应和请求类型。
//!
//! 所有 Fetcher 模式（Http / Dynamic / Stealth）返回同一个 `Response`，
//! 用户无需关心底层实现即可使用 `.css()` / `.json()` 等 API。
//! Spider 引擎也复用同一套 Request/Response，避免类型重复。

mod method;
mod request;
mod response;

pub use method::{FetchMode, Method};
pub use request::Request;
pub use response::Response;

pub use response::ResponseParts;
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
}
