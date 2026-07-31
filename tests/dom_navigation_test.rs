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
