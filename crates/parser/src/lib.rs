//! HTML parsing with CSS selectors.

pub mod adaptive;
pub mod difflib;
pub mod document;
pub mod generate;
mod node;
mod response_ext;

pub use adaptive::{relocate_with_snapshot, similarity, ElementSnapshot, DEFAULT_TOLERANCE};
pub use node::{Node, NodeList};
pub use response_ext::ResponseExt;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_html_and_text() {
        let doc = Node::from_html("<html><body><h1>Hello World</h1></body></html>");
        assert!(doc.text().contains("Hello World"));
    }

    #[test]
    fn test_select() {
        let doc = Node::from_html(
            r#"<html><body>
            <div class="item">First</div>
            <div class="item">Second</div>
        </body></html>"#,
        );
        let items = doc.select("div.item");
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_select_one() {
        let doc = Node::from_html(
            r#"<html><body>
            <p id="main">Content here</p>
        </body></html>"#,
        );
        let p = doc.select_one("#main");
        assert!(p.is_some());
        assert!(p.unwrap().text().contains("Content here"));
    }

    #[test]
    fn test_attr() {
        let node = Node::from_fragment(r#"<a href="https://example.com" class="link">Click</a>"#);
        assert_eq!(node.attr("href"), Some("https://example.com".to_string()));
        assert_eq!(node.attr("class"), Some("link".to_string()));
        assert_eq!(node.attr("nonexistent"), None);
    }

    #[test]
    fn test_attrs() {
        let node = Node::from_fragment(r#"<div id="test" data-x="1">Content</div>"#);
        let attrs = node.attrs();
        assert_eq!(attrs.get("id"), Some(&"test".to_string()));
        assert_eq!(attrs.get("data-x"), Some(&"1".to_string()));
    }

    #[test]
    fn test_html() {
        let node = Node::from_fragment(r#"<div><span>inner</span></div>"#);
        let html = node.html();
        assert!(html.contains("<span>inner</span>"));
    }

    #[test]
    fn test_outer_html() {
        let node = Node::from_fragment(r#"<div class="x">text</div>"#);
        let outer = node.outer_html();
        assert!(outer.contains("class=\"x\""));
    }

    #[test]
    fn test_contains_text() {
        let doc = Node::from_html("<html><body><p>Hello World</p></body></html>");
        assert!(doc.contains_text("Hello"));
        assert!(!doc.contains_text("Goodbye"));
    }

    #[test]
    fn test_node_list_text() {
        let doc = Node::from_html(
            r#"<html><body>
            <li>A</li><li>B</li><li>C</li>
        </body></html>"#,
        );
        let texts = doc.select("li").text();
        assert_eq!(texts, vec!["A", "B", "C"]);
    }

    #[test]
    fn test_node_list_filter() {
        let doc = Node::from_html(
            r#"<html><body>
            <div class="keep">Keep1</div>
            <div class="drop">Drop</div>
            <div class="keep">Keep2</div>
        </body></html>"#,
        );
        let filtered = doc
            .select("div")
            .filter(|n| n.attr("class") == Some("keep".to_string()));
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_generate_selector() {
        let node = Node::from_fragment(r#"<div id="unique">Content</div>"#);
        assert_eq!(node.generate_selector(), "#unique");
    }

    #[test]
    fn test_from_fragment_table_element() {
        // 表格元素片段不应被强制包裹 <table>（Important 2 回归测试）
        // 旧 Task 3 重构后用 parse_document 会让 tag() 返回 "table"；
        // 修复后用 parse_fragment，tag() 应返回 "td"。
        let node = Node::from_fragment("<td>cell</td>");
        assert_eq!(node.tag(), "td");
        assert!(node.text().contains("cell"));
        assert!(node.outer_html().contains("<td>cell</td>"));
    }

    #[test]
    fn select_invalid_selector_returns_empty_not_all() {
        let doc = Node::from_html(r#"<html><body><p>a</p><p>b</p></body></html>"#);
        // 非法选择器（未闭合括号）
        let nodes = doc.select("p[onclick=alert(");
        assert!(
            nodes.iter().count() == 0,
            "非法选择器应返回空，实际返回 {} 个（静默回退到 * 会返回 2 个 <p>）",
            nodes.iter().count()
        );
    }

    #[test]
    fn select_valid_selector_still_works() {
        let doc = Node::from_html(r#"<html><body><p>a</p><p>b</p></body></html>"#);
        let nodes = doc.select("p");
        assert_eq!(nodes.iter().count(), 2);
    }
}
