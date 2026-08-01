//! `Response` 的 HTML 解析扩展：core DTO 不依赖 parser，解析能力由本 trait 提供。

use crate::{Node, NodeList};
use wisp_core::Response;

/// 在 `Response` 上解析 HTML 的扩展接口。
pub trait ResponseExt {
    /// 解析 HTML 为文档节点。
    fn parse(&self) -> Node;
    /// CSS 选择器查询。
    fn css(&self, selector: &str) -> NodeList;
    /// CSS 选择器查询第一个匹配元素。
    fn select_one(&self, selector: &str) -> Option<Node>;
    /// 按文本内容查找元素。
    fn find_by_text(&self, text: &str, tag: Option<&str>, exact: bool) -> NodeList;
}

impl ResponseExt for Response {
    fn parse(&self) -> Node {
        let text = wisp_core::encoding::decode_borrowed(&self.body, &self.content_type);
        Node::from_html_owned(text.into_owned())
    }

    fn css(&self, selector: &str) -> NodeList {
        self.parse().select(selector)
    }

    fn select_one(&self, selector: &str) -> Option<Node> {
        self.parse().select_one(selector)
    }

    fn find_by_text(&self, text: &str, tag: Option<&str>, exact: bool) -> NodeList {
        self.parse().find_by_text(text, tag, exact)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use wisp_core::{Request, Response};

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
    fn response_ext_css_works() {
        let resp = make_response(r#"<div class="item">A</div><div class="item">B</div>"#);
        assert_eq!(resp.css(".item").len(), 2);
    }

    #[test]
    fn response_ext_parse_then_query() {
        let resp = make_response("<h1>Title</h1>");
        let doc = resp.parse();
        assert_eq!(
            doc.select_one("h1").map(|n| n.text()),
            Some("Title".to_string())
        );
    }

    #[test]
    fn response_ext_select_one() {
        let resp = make_response(r#"<p id="main">Content</p>"#);
        assert_eq!(
            resp.select_one("#main").map(|n| n.text()),
            Some("Content".to_string())
        );
    }

    #[test]
    fn response_ext_find_by_text() {
        let resp = make_response(r#"<div>Apple</div><div>Banana</div>"#);
        assert_eq!(resp.find_by_text("Apple", Some("div"), true).len(), 1);
    }
}
