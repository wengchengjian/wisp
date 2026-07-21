//! Verify XPath queries work for both simple (fast path) and complex (sxd) expressions.

use wisp::parser::Node;

const XPATH_HTML: &str = r#"
<html>
  <body>
    <div id="main" class="container">
      <h1>Title</h1>
      <ul>
        <li class="item">Item 1</li>
        <li class="item">Item 2</li>
        <li class="item">Item 3</li>
        <li class="special">Item 4</li>
      </ul>
      <a href="https://example.com/page1">Link 1</a>
      <a href="https://example.com/page2">Link 2</a>
    </div>
  </body>
</html>
"#;

#[test]
fn test_xpath_simple_tag() {
    // 快速路径：//tag -> tag
    let doc = Node::from_html(XPATH_HTML);
    let lis = doc.xpath("//li");
    assert_eq!(lis.len(), 4);
}

#[test]
fn test_xpath_by_id() {
    // 快速路径：//*[@id='value'] -> #value
    let doc = Node::from_html(XPATH_HTML);
    let main = doc.xpath("//*[@id='main']");
    assert_eq!(main.len(), 1);
    assert_eq!(main.get(0).unwrap().attr("class"), Some("container".to_string()));
}

#[test]
fn test_xpath_attr_value() {
    // 快速路径：//tag[@attr='value']
    let doc = Node::from_html(XPATH_HTML);
    let special = doc.xpath("//li[@class='special']");
    assert_eq!(special.len(), 1);
    assert!(special.get(0).unwrap().text().contains("Item 4"));
}

#[test]
fn test_xpath_contains_href() {
    // 快速路径：//tag[contains(@attr, 'value')]
    let doc = Node::from_html(XPATH_HTML);
    let links = doc.xpath("//a[contains(@href, 'example.com')]");
    assert_eq!(links.len(), 2);
}

#[test]
fn test_xpath_position_predicate() {
    // 慢路径：position() 谓词需要 sxd-xpath
    let doc = Node::from_html(XPATH_HTML);
    let items = doc.xpath("//li[position()>2]");
    assert_eq!(items.len(), 2);
}

#[test]
fn test_xpath_text_content() {
    // 慢路径：text() 函数
    let doc = Node::from_html(XPATH_HTML);
    let items = doc.xpath("//li[contains(text(), 'Item 1')]");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_xpath_returns_empty_on_no_match() {
    let doc = Node::from_html(XPATH_HTML);
    let result = doc.xpath("//nonexistent");
    assert_eq!(result.len(), 0);
}

#[test]
fn test_xpath_malformed_returns_empty() {
    let doc = Node::from_html(XPATH_HTML);
    // 格式错误的 xpath 应返回空，不 panic
    let result = doc.xpath("///[[[");
    assert_eq!(result.len(), 0);
}

#[test]
fn test_xpath_html5_tolerance() {
    // 不规范 HTML（未闭合标签）应能正常解析
    let html = r#"<html><body><div><p>Unclosed paragraph<div>Nested</div></body></html>"#;
    let doc = Node::from_html(html);
    let result = doc.xpath("//p");
    assert_eq!(result.len(), 1);
    assert!(result.get(0).unwrap().text().contains("Unclosed"));
}
