use super::*;
use std::sync::Arc;
use wisp_parser::{DEFAULT_TOLERANCE, ElementSnapshot, Node, relocate_with_snapshot};
use wisp_storage::{MemoryStore, Store, load_element, save_element};

use super::convert::{row_to_snapshot, snapshot_to_row};

fn make_doc() -> Node {
    Node::from_html(
        r#"<html><body>
            <div class="item" id="main">Hello World</div>
        </body></html>"#,
    )
}

fn make_store() -> Arc<dyn Store> {
    Arc::new(MemoryStore::default())
}

#[tokio::test]
async fn css_adaptive_returns_node_when_css_matches() {
    let tracker = AdaptiveTracker::new(make_store());
    let doc = make_doc();

    let found = tracker
        .css_adaptive(
            &doc,
            "#main",
            "main-key",
            "https://example.com",
            true,
            DEFAULT_TOLERANCE,
        )
        .await
        .expect("css_adaptive 应成功");

    assert!(found.is_some());
    assert_eq!(found.unwrap().text(), "Hello World");
}

#[tokio::test]
async fn css_adaptive_returns_none_when_no_css_and_no_snapshot() {
    let tracker = AdaptiveTracker::new(make_store());
    let doc = make_doc();

    let found = tracker
        .css_adaptive(
            &doc,
            "#nonexistent",
            "missing-key",
            "https://example.com",
            false,
            DEFAULT_TOLERANCE,
        )
        .await
        .expect("css_adaptive 应成功");

    assert!(found.is_none());
}

#[tokio::test]
async fn css_adaptive_relocates_from_saved_snapshot() {
    let store = make_store();
    let tracker = AdaptiveTracker::new(Arc::clone(&store));

    // 第一次：CSS 匹配 #main，auto_save 保存快照
    let doc1 = make_doc();
    let found1 = tracker
        .css_adaptive(
            &doc1,
            "#main",
            "relocate-key",
            "https://example.com",
            true,
            DEFAULT_TOLERANCE,
        )
        .await
        .expect("css_adaptive 应成功");
    assert!(found1.is_some());

    // 第二次：CSS 不匹配（#missing-id），从存储加载快照重定位到 #main
    let doc2 = make_doc();
    let found2 = tracker
        .css_adaptive(
            &doc2,
            "#missing-id",
            "relocate-key",
            "https://example.com",
            false,
            DEFAULT_TOLERANCE,
        )
        .await
        .expect("css_adaptive 应成功");
    assert!(found2.is_some());
    assert_eq!(found2.unwrap().text(), "Hello World");
}

#[tokio::test]
async fn css_adaptive_default_uses_default_tolerance() {
    let tracker = AdaptiveTracker::new(make_store());
    let doc = make_doc();

    let found = tracker
        .css_adaptive_default(&doc, "#main", "k", "https://x", false)
        .await
        .expect("css_adaptive_default 应成功");
    assert!(found.is_some());
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

    let snapshot = ElementSnapshot::capture(&apple_node);
    let key = "product-name";
    let url = "https://example.com/products";
    save_element(store.as_ref(), url, key, &snapshot_to_row(snapshot, 0))
        .await
        .unwrap();

    let loaded = load_element(store.as_ref(), url, key)
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
    assert_eq!(found.unwrap().text(), "Apple");
}

#[test]
fn test_relocate_returns_none_when_no_match() {
    let doc = Node::from_html(HTML_BEFORE);
    let apple = doc.select_one(".name").unwrap();
    let snapshot = ElementSnapshot::capture(&apple);

    let other_html = r#"<html><body><footer><p>copyright</p></footer></body></html>"#;
    let other_doc = Node::from_html(other_html);

    let found = relocate_with_snapshot(&other_doc, &snapshot, 0.99);
    assert!(found.is_none(), "should not find a match in unrelated HTML");
}

#[tokio::test]
async fn test_relocate_finds_best_match_among_candidates() {
    let store = make_store();
    let doc = Node::from_html(HTML_BEFORE);
    let banana = doc.select_all(".name").into_iter().nth(1).unwrap();
    let snapshot = ElementSnapshot::capture(&banana);
    save_element(store.as_ref(), "u", "k", &snapshot_to_row(snapshot, 0))
        .await
        .unwrap();

    let doc2 = Node::from_html(HTML_BEFORE);
    let loaded = load_element(store.as_ref(), "u", "k")
        .await
        .unwrap()
        .unwrap();
    let loaded_snap = row_to_snapshot(loaded);
    let found = relocate_with_snapshot(&doc2, &loaded_snap, 0.3).unwrap();
    assert_eq!(found.text(), "Banana");
}
