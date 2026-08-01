//! Adaptive relocation tests: capture snapshot, simulate site change, verify relocate finds the right element.

use std::sync::Arc;
use wisp::crawl::adaptive::{row_to_snapshot, snapshot_to_row};
use wisp::crawl::AdaptiveTracker;
use wisp::parser::{
    adaptive::{relocate_with_snapshot, ElementSnapshot, DEFAULT_TOLERANCE},
    Node,
};
use wisp::storage::{MemoryStore, Store};

fn make_store() -> impl Store {
    MemoryStore::default()
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

#[tokio::test]
async fn test_capture_then_relocate_after_class_change() {
    let store = make_store();
    let doc_before = Node::from_html(HTML_BEFORE);
    let apple_node = doc_before.select_one(".name").expect("should find .name");

    // Capture snapshot of the first .name element
    let snapshot = ElementSnapshot::capture(&apple_node);
    let key = "product-name";
    let url = "https://example.com/products";
    wisp::storage::save_element(&store, url, key, &snapshot_to_row(snapshot, 0))
        .await
        .unwrap();

    // Simulate site redesign: .name → .title, parent ul.list → ul.items
    let loaded = wisp::storage::load_element(&store, url, key)
        .await
        .unwrap()
        .unwrap();
    let loaded_snapshot = row_to_snapshot(loaded);

    let doc_after = Node::from_html(HTML_AFTER);
    let found = relocate_with_snapshot(&doc_after, &loaded_snapshot, DEFAULT_TOLERANCE);

    assert!(
        found.is_some(),
        "should relocate the element after site change"
    );
    let found = found.unwrap();
    assert_eq!(
        found.text(),
        "Apple",
        "relocated element should contain the right text"
    );
}

#[test]
fn test_relocate_returns_none_when_no_match() {
    let doc = Node::from_html(HTML_BEFORE);
    let apple = doc.select_one(".name").unwrap();
    let snapshot = ElementSnapshot::capture(&apple);

    // Totally different HTML with no similar elements
    let other_html = r#"<html><body><footer><p>copyright</p></footer></body></html>"#;
    let other_doc = Node::from_html(other_html);

    let found = relocate_with_snapshot(&other_doc, &snapshot, 0.99); // high tolerance
    assert!(found.is_none(), "should not find a match in unrelated HTML");
}

#[tokio::test]
async fn test_relocate_finds_best_match_among_candidates() {
    let store = make_store();
    let doc = Node::from_html(HTML_BEFORE);
    let banana = doc.select_all(".name").into_iter().nth(1).unwrap();
    let snapshot = ElementSnapshot::capture(&banana);
    wisp::storage::save_element(&store, "u", "k", &snapshot_to_row(snapshot, 0))
        .await
        .unwrap();

    // Re-parse same HTML - should find Banana (not Apple)
    let doc2 = Node::from_html(HTML_BEFORE);
    let loaded = wisp::storage::load_element(&store, "u", "k")
        .await
        .unwrap()
        .unwrap();
    let loaded_snap = row_to_snapshot(loaded);
    let found = relocate_with_snapshot(&doc2, &loaded_snap, 0.3).unwrap();
    assert_eq!(found.text(), "Banana");
}

#[tokio::test]
async fn test_css_adaptive_falls_back_to_snapshot() {
    let store: Arc<dyn Store> = Arc::new(make_store());
    let url = "https://example.com/p";
    let tracker = AdaptiveTracker::new(Arc::clone(&store));

    // First call: CSS works, snapshot is auto-saved
    let doc_before = Node::from_html(HTML_BEFORE);
    let found = tracker
        .css_adaptive(&doc_before, ".name", "name-key", url, true, 0.5)
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().text(), "Apple");

    // Verify snapshot was saved via the same store the tracker holds
    let row = wisp::storage::load_element(&*store, url, "name-key")
        .await
        .unwrap();
    assert!(row.is_some());

    // Second call: CSS fails (.name not in HTML_AFTER), should relocate via snapshot
    let doc_after = Node::from_html(HTML_AFTER);
    let found = tracker
        .css_adaptive(&doc_after, ".name", "name-key", url, true, 0.5)
        .await
        .unwrap();
    assert!(found.is_some(), "css_adaptive should relocate via snapshot");
    assert_eq!(found.unwrap().text(), "Apple");
}

#[tokio::test]
async fn test_css_adaptive_returns_none_when_no_snapshot_and_css_fails() {
    let store: Arc<dyn Store> = Arc::new(make_store());
    let tracker = AdaptiveTracker::new(Arc::clone(&store));
    let doc = Node::from_html(HTML_BEFORE);
    let found = tracker
        .css_adaptive(&doc, ".nonexistent", "missing-key", "url", false, 0.5)
        .await
        .unwrap();
    assert!(found.is_none());
}
