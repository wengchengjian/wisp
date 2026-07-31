# Task 4: DOM 导航真实实现（parent/children/sibling/ancestors/matches）

**Files:**
- Modify: `src/parser/mod.rs`
- Create: `tests/dom_navigation_test.rs`

**TDD:** 先写失败测试，验证失败，再实现，验证通过。

## Step 1: 先写失败测试 tests/dom_navigation_test.rs

```rust
//! Verify DOM navigation methods work correctly after Node refactor.

use wisp::parser::Node;

const NAV_HTML: &str = r#"
<html>
  <body>
    <div id="parent" class="container">
      <p class="first">First paragraph</p>
      <p class="second" data-x="1">Second paragraph</p>
      <p class="third">Third paragraph</p>
      <span class="sibling">After paragraphs</span>
    </div>
    <section id="other">Other section</section>
  </body>
</html>
"#;

#[test]
fn test_parent_navigation() {
    let doc = Node::from_html(NAV_HTML);
    let p = doc.select_one("p.second").expect("p.second should exist");
    let parent = p.parent().expect("p should have a parent");
    assert_eq!(parent.attr("id"), Some("parent".to_string()));
    assert_eq!(parent.tag(), "div");
}

#[test]
fn test_children_navigation() {
    let doc = Node::from_html(NAV_HTML);
    let parent = doc.select_one("#parent").expect("#parent should exist");
    let children = parent.children();
    // 3 个 <p> + 1 个 <span> = 4 个元素子节点
    assert_eq!(children.len(), 4);
    assert_eq!(children.get(0).unwrap().attr("class"), Some("first".to_string()));
    assert_eq!(children.get(3).unwrap().tag(), "span");
}

#[test]
fn test_next_sibling() {
    let doc = Node::from_html(NAV_HTML);
    let first = doc.select_one("p.first").expect("p.first should exist");
    let next = first.next_sibling().expect("should have next sibling");
    assert_eq!(next.attr("class"), Some("second".to_string()));
}

#[test]
fn test_prev_sibling() {
    let doc = Node::from_html(NAV_HTML);
    let third = doc.select_one("p.third").expect("p.third should exist");
    let prev = third.prev_sibling().expect("should have prev sibling");
    assert_eq!(prev.attr("class"), Some("second".to_string()));
}

#[test]
fn test_next_sibling_none_at_end() {
    let doc = Node::from_html(NAV_HTML);
    let span = doc.select_one("span.sibling").expect("span should exist");
    // span 是 div 的最后一个元素子节点
    assert!(span.next_sibling().is_none());
}

#[test]
fn test_ancestors_iterator() {
    let doc = Node::from_html(NAV_HTML);
    let p = doc.select_one("p.first").expect("p.first should exist");
    let ancestors: Vec<Node> = p.ancestors().collect();
    // p -> div#parent -> body -> html
    assert!(ancestors.len() >= 3);
    assert_eq!(ancestors[0].attr("id"), Some("parent".to_string()));
    assert_eq!(ancestors[1].tag(), "body");
    assert_eq!(ancestors[2].tag(), "html");
}

#[test]
fn test_matches_simple_selector() {
    let doc = Node::from_html(NAV_HTML);
    let p = doc.select_one("p.second").expect("p.second should exist");
    assert!(p.matches("p"));
    assert!(p.matches("p.second"));
    assert!(p.matches("[data-x]"));
    assert!(!p.matches("p.first"));
    assert!(!p.matches("div"));
}

#[test]
fn test_matches_compound_selector() {
    let doc = Node::from_html(NAV_HTML);
    let p = doc.select_one("p.second").expect("p.second should exist");
    assert!(p.matches("p.second[data-x='1']"));
    assert!(!p.matches("p.second[data-x='2']"));
}

#[test]
fn test_tag_name() {
    let doc = Node::from_html(NAV_HTML);
    let p = doc.select_one("p.first").expect("p.first should exist");
    assert_eq!(p.tag(), "p");
    let div = doc.select_one("#parent").expect("#parent should exist");
    assert_eq!(div.tag(), "div");
}
```

## Step 2: 运行测试验证失败

Run: `cargo test --test dom_navigation_test`
Expected: 多个测试 FAIL（parent/ancestors/matches 未实现，Task 3 临时返回 None/false）

注意：test_children_navigation 和 test_tag_name 可能已经通过（Task 3 已实现 children() 和 tag()）。test_parent_navigation / test_next_sibling / test_prev_sibling / test_ancestors_iterator / test_matches_* 会失败。

## Step 3: 真实实现 parent / next_sibling / prev_sibling

在 `src/parser/mod.rs` 的 Node impl 中替换临时实现：

```rust
    /// Get the parent element.
    pub fn parent(&self) -> Option<Node> {
        let element = self.element_ref()?;
        element.parent()
            .and_then(ElementRef::wrap)
            .filter(|p| p.value().is_element())
            .map(|p| Node::from_element_ref(self.doc.clone(), p))
    }

    /// Get the next sibling element (skips non-element nodes).
    pub fn next_sibling(&self) -> Option<Node> {
        let element = self.element_ref()?;
        let mut sib = element.next_sibling();
        while let Some(s) = sib {
            if let Some(el) = ElementRef::wrap(s) {
                if el.value().is_element() {
                    return Some(Node::from_element_ref(self.doc.clone(), el));
                }
            }
            sib = s.next_sibling();
        }
        None
    }

    /// Get the previous sibling element (skips non-element nodes).
    pub fn prev_sibling(&self) -> Option<Node> {
        let element = self.element_ref()?;
        let mut sib = element.prev_sibling();
        while let Some(s) = sib {
            if let Some(el) = ElementRef::wrap(s) {
                if el.value().is_element() {
                    return Some(Node::from_element_ref(self.doc.clone(), el));
                }
            }
            sib = s.prev_sibling();
        }
        None
    }
```

**注意 scraper 0.23 API**：`element.parent()` 返回 `Option<NodeRef>`，`ElementRef::wrap(node_ref)` 返回 `Option<ElementRef>`。`element.value().is_element()` 可能不存在——Task 3 发现 `Element` 没有 `is_element()` 方法。如果编译失败，用 `ElementRef::wrap(s).is_some()` 替代 `.filter(|p| p.value().is_element())`，因为 `ElementRef::wrap` 本身已经过滤非元素节点。

备选实现（如果上面编译失败）：
```rust
    pub fn parent(&self) -> Option<Node> {
        let element = self.element_ref()?;
        element.parent()
            .and_then(ElementRef::wrap)
            .map(|p| Node::from_element_ref(self.doc.clone(), p))
    }

    pub fn next_sibling(&self) -> Option<Node> {
        let element = self.element_ref()?;
        let mut sib = element.next_sibling();
        while let Some(s) = sib {
            if let Some(el) = ElementRef::wrap(s) {
                return Some(Node::from_element_ref(self.doc.clone(), el));
            }
            sib = s.next_sibling();
        }
        None
    }

    pub fn prev_sibling(&self) -> Option<Node> {
        let element = self.element_ref()?;
        let mut sib = element.prev_sibling();
        while let Some(s) = sib {
            if let Some(el) = ElementRef::wrap(s) {
                return Some(Node::from_element_ref(self.doc.clone(), el));
            }
            sib = s.prev_sibling();
        }
        None
    }
```
（ElementRef::wrap 只对元素节点返回 Some，所以不需要额外 is_element 检查）

## Step 4: 新增 ancestors() 迭代器

在 Node impl 中追加：

```rust
    /// 从当前节点的父节点开始，向上迭代到文档根。
    pub fn ancestors(&self) -> impl Iterator<Item = Node> + '_ {
        std::iter::successors(self.parent(), |node| node.parent())
    }
```

## Step 5: 真实实现 matches()

```rust
    /// Check if element matches a CSS selector.
    pub fn matches(&self, css: &str) -> bool {
        let selector = match CssSelector::parse(css) {
            Ok(s) => s,
            Err(_) => return false,
        };
        self.element_ref()
            .map(|e| e.value().matches(&selector))
            .unwrap_or(false)
    }
```

**注意**：`e.value()` 返回 `&Element`，scraper 0.23 的 `Element` 有 `matches(&Selector) -> bool` 方法。如果编译失败，查 scraper 0.23 文档确认 matches 方法签名。

## Step 6: 运行 cargo check 验证编译

Run: `cargo check`
Expected: 编译通过

## Step 7: 运行新测试验证通过

Run: `cargo test --test dom_navigation_test`
Expected: 9 个测试全部 PASS

## Step 8: 运行现有测试确保未破坏

Run: `cargo test --lib && cargo test --test adaptive_test && cargo test --test difflib_test`
Expected: 全部通过（35 lib + 5 adaptive + 7 difflib）

## Step 9: 提交

```bash
git add src/parser/mod.rs tests/dom_navigation_test.rs
git commit -m "feat: 真实实现 parent/children/sibling/ancestors/matches DOM 导航"
```
