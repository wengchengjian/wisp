use super::*;
use serde::Serialize;
use wisp_fetcher::{Request, Response};

fn make_page() -> Page {
    let resp = Response::from_http(
        200,
        "http://example.com/page".into(),
        std::collections::HashMap::new(),
        b"<html><body><h1>hi</h1></body></html>".to_vec(),
        String::new(),
        Request::get("http://example.com/page"),
    );
    Page::new(resp)
}

#[test]
fn item_serialization_failure_is_not_silently_null() {
    let mut page = make_page();
    // 自定义类型在 serialize 时返回错误，触发 to_value 失败路径。
    struct BadSerialize;
    impl Serialize for BadSerialize {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("boom"))
        }
    }
    page.item(BadSerialize);
    assert!(page.items().is_empty(), "序列化失败不应写入 Value::Null");
}

#[test]
fn item_serialization_success_still_collects() {
    let mut page = make_page();
    page.item(serde_json::json!({ "title": "hello" }));
    assert_eq!(page.items().len(), 1);
    assert_eq!(page.items()[0]["title"], "hello");
}

#[tokio::test]
async fn follow_links_filtered_matches_url_predicate() {
    let resp = Response::from_http(
        200,
        "http://example.com/page".into(),
        std::collections::HashMap::new(),
        r#"<a href="/blog/1">b</a><a href="/other">o</a>"#.as_bytes().to_vec(),
        "text/html".into(),
        Request::get("http://example.com/page"),
    );
    let mut page = Page::new(resp);
    page.follow_links_filtered(
        &["a[href]"],
        "detail",
        |url| url.contains("/blog/"),
        |_page, _idx, _a| serde_json::json!(null),
    )
    .await;
    let follows = page.follows();
    assert_eq!(follows.len(), 1);
    assert_eq!(follows[0].url, "http://example.com/blog/1");
}
