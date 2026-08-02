use super::*;
use std::sync::Arc;
use wisp_parser::{DEFAULT_TOLERANCE, Node};
use wisp_storage::{MemoryStore, Store};

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
