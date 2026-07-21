//! Adaptive relocation tests: capture snapshot, simulate site change, verify relocate finds the right element.

use wisp::parser::{Node, adaptive::{ElementSnapshot, relocate_with_snapshot, DEFAULT_TOLERANCE}};
use wisp::storage::Store;

fn make_store() -> Store {
    Store::open_in_memory().unwrap()
}

const HTML_BEFORE: &str = r#"
<html><body>
<div class="products">
  <ul class="list">
    <li class="item"><span class="name">Apple</span><span class="price">$1</span></li>
    <li class="item"><span class="name">Banana</span><span class="price">$2</span></li>
  </ul>
</div>
</body></html>
"#;

const HTML_AFTER: &str = r#"
<html><body>
<div class="product-list-v2">
  <ul class="items">
    <li class="row"><span class="title">Apple</span><span class="cost">$1</span></li>
    <li class="row"><span class="title">Banana</span><span class="cost">$2</span></li>
  </ul>
</div>
</body></html>
"#;

#[test]
fn test_capture_then_relocate_after_class_change() {
    let store = make_store();
    let doc_before = Node::from_html(HTML_BEFORE);
    let apple_node = doc_before.select_one(".name").expect("should find .name");

    // Capture snapshot of the first .name element
    let snapshot = ElementSnapshot::capture(&apple_node);
    let key = "product-name";
    let url = "https://example.com/products";
    store.save_element(url, key, &snapshot.to_row(0)).unwrap();

    // Simulate site redesign: .name → .title, parent ul.list → ul.items
    let loaded = store.load_element(url, key).unwrap().unwrap();
    let loaded_snapshot = ElementSnapshot::from_row(loaded);

    let doc_after = Node::from_html(HTML_AFTER);
    let found = relocate_with_snapshot(&doc_after, &loaded_snapshot, DEFAULT_TOLERANCE);

    assert!(found.is_some(), "should relocate the element after site change");
    let found = found.unwrap();
    assert_eq!(found.text(), "Apple", "relocated element should contain the right text");
}

#[test]
fn test_relocate_returns_none_when_no_match() {
    let store = make_store();
    let doc = Node::from_html(HTML_BEFORE);
    let apple = doc.select_one(".name").unwrap();
    let snapshot = ElementSnapshot::capture(&apple);

    // Totally different HTML with no similar elements
    let other_html = r#"<html><body><footer><p>copyright</p></footer></body></html>"#;
    let other_doc = Node::from_html(other_html);

    let found = relocate_with_snapshot(&other_doc, &snapshot, 0.99);  // high tolerance
    assert!(found.is_none(), "should not find a match in unrelated HTML");
}

#[test]
fn test_relocate_finds_best_match_among_candidates() {
    let store = make_store();
    let doc = Node::from_html(HTML_BEFORE);
    let banana = doc.select_all(".name").into_iter().nth(1).unwrap();
    let snapshot = ElementSnapshot::capture(&banana);
    store.save_element("u", "k", &snapshot.to_row(0)).unwrap();

    // Re-parse same HTML - should find Banana (not Apple)
    let doc2 = Node::from_html(HTML_BEFORE);
    let loaded = store.load_element("u", "k").unwrap().unwrap();
    let loaded_snap = ElementSnapshot::from_row(loaded);
    let found = relocate_with_snapshot(&doc2, &loaded_snap, 0.3).unwrap();
    assert_eq!(found.text(), "Banana");
}
