# Task 6: XPath 测试

**Files:**
- Create: `tests/xpath_test.rs`

## Step 1: 创建 tests/xpath_test.rs

```rust
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
```

## Step 2: 运行测试

Run: `cargo test --test xpath_test`
Expected: 9 个测试通过。如果 `test_xpath_position_predicate` / `test_xpath_text_content` 失败，检查 sxd-xpath 集成是否正确。

**重要**：慢路径测试（test_xpath_position_predicate, test_xpath_text_content）依赖 Task 5 的 sxd-xpath 集成。如果这两个测试失败，可能是：
1. sxd-xpath 的 position() 或 text() 函数支持问题
2. locate_in_sxd 定位错误（context 节点不对）
3. find_in_scraper 回查失败（sxd 节点找不到对应 scraper 节点）

如果慢路径测试失败，先分析失败原因。如果是 Task 5 实现的 bug，修复 src/parser/xpath.rs。如果是 sxd-xpath 本身的限制（比如 position() 语义不同），调整测试期望或标记测试为 #[ignore] 并说明原因。

## Step 3: 运行现有测试确保未破坏

Run: `cargo test --lib && cargo test --test dom_navigation_test && cargo test --test adaptive_test`
Expected: 全部通过

## Step 4: 提交

```bash
git add tests/xpath_test.rs
git commit -m "test: XPath 快速路径与 sxd-xpath 慢路径覆盖测试"
```
