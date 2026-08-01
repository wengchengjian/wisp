use super::*;
use crate::Spider;
use crate::{Request, Response};
use serde_json::json;
use wisp_parser::ResponseExt;

#[test]
fn test_spider_builder_basic() {
    let spider = SpiderBuilder::new("test")
        .start_urls(vec!["https://example.com/"])
        .on("default", |_resp| async move {
            (vec![json!({ "ok": true })], vec![])
        })
        .build();

    assert_eq!(spider.name(), "test");
    assert_eq!(spider.start_urls(), vec!["https://example.com/"]);
    // ND-031-ARCH：download_delay/obey_robots 已迁移到 EngineBuilder
}

#[test]
fn test_spider_builder_allowed_domains() {
    let spider = SpiderBuilder::new("test")
        .start_urls(vec!["https://example.com/"])
        .allowed_domains(vec!["example.com"])
        .on("default", |_| async move { (vec![], vec![]) })
        .build();

    let domains = spider.allowed_domains();
    assert!(domains.contains("example.com"));
}

#[test]
#[should_panic(expected = "必须至少注册一个 handler")]
fn test_spider_builder_no_handler_panics() {
    let _spider = SpiderBuilder::new("test")
        .start_urls(vec!["https://example.com/"])
        .build();
}

#[tokio::test]
async fn test_closure_spider_default_handler() {
    let spider = SpiderBuilder::new("test")
        .start_urls(vec!["https://example.com/"])
        .on("default", |resp| async move {
            let doc = resp.parse();
            let title = doc.select_one("h1").map(|n| n.text()).unwrap_or_default();
            (vec![json!({ "title": title })], vec![])
        })
        .build();

    let resp = Response::from_http(
        200,
        "https://example.com/".into(),
        Default::default(),
        b"<html><body><h1>Hello</h1></body></html>".to_vec(),
        String::new(),
        Request::get("https://example.com/"),
    );

    let (items, follows) = spider.handle(resp).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Hello");
    assert!(follows.is_empty());
}

#[tokio::test]
async fn test_closure_spider_async_handler() {
    let spider = SpiderBuilder::new("async-test")
        .start_urls(vec!["https://example.com/"])
        .on("default", |resp| async move {
            let doc = resp.parse();
            let text = doc.select_one("p").map(|n| n.text()).unwrap_or_default();
            (vec![json!({ "text": text })], vec![])
        })
        .build();

    let resp = Response::from_http(
        200,
        "https://example.com/".into(),
        Default::default(),
        b"<html><body><p>World</p></body></html>".to_vec(),
        String::new(),
        Request::get("https://example.com/"),
    );

    let (items, _) = spider.handle(resp).await;
    assert_eq!(items[0]["text"], "World");
}

#[test]
fn test_closure_spider_custom_is_blocked() {
    let spider = SpiderBuilder::new("test")
        .start_urls(Vec::<String>::new())
        .on("default", |_| async move { (vec![], vec![]) })
        .is_blocked(|resp| resp.body.windows(7).any(|w| w == b"blocked"))
        .build();

    let resp = Response::from_http(
        200,
        "http://x.com".into(),
        Default::default(),
        b"you are blocked".to_vec(),
        String::new(),
        Request::get("http://x.com"),
    );
    assert!(spider.is_blocked(&resp));

    let ok_resp = Response::from_http(
        200,
        "http://x.com".into(),
        Default::default(),
        b"welcome".to_vec(),
        String::new(),
        Request::get("http://x.com"),
    );
    assert!(!spider.is_blocked(&ok_resp));
}

#[tokio::test]
async fn test_closure_spider_handle_routes_by_callback() {
    // 验证 handle() 根据 callback label 路由分发
    let spider = SpiderBuilder::new("routing")
        .start_urls(vec!["https://example.com/"])
        .on("default", |_resp| async move {
            (vec![json!({ "handler": "default" })], vec![])
        })
        .on("detail", |_resp| async move {
            (vec![json!({ "handler": "detail" })], vec![])
        })
        .on("content", |resp| async move {
            let title = resp.css("h1").text().join("");
            (
                vec![json!({ "handler": "content", "title": title })],
                vec![],
            )
        })
        .build();

    // 1. callback=None → default handler
    let resp_default = Response::from_http(
        200,
        "https://example.com/".into(),
        Default::default(),
        b"<html></html>".to_vec(),
        String::new(),
        Request::get("https://example.com/"),
    );
    let (items, _) = spider.handle(resp_default).await;
    assert_eq!(items[0]["handler"], "default");

    // 2. callback="detail" → detail handler
    let resp_detail = Response::from_http(
        200,
        "https://example.com/detail/1".into(),
        Default::default(),
        b"<html></html>".to_vec(),
        String::new(),
        Request::get("https://example.com/detail/1").with_callback("detail"),
    );
    let (items, _) = spider.handle(resp_detail).await;
    assert_eq!(items[0]["handler"], "detail");

    // 3. callback="content" → content handler
    let resp_content = Response::from_http(
        200,
        "https://example.com/content/1".into(),
        Default::default(),
        b"<html><h1>Title</h1></html>".to_vec(),
        String::new(),
        Request::get("https://example.com/content/1").with_callback("content"),
    );
    let (items, _) = spider.handle(resp_content).await;
    assert_eq!(items[0]["handler"], "content");
    assert_eq!(items[0]["title"], "Title");

    // 4. callback="unknown" → 回退到 default handler
    let resp_unknown = Response::from_http(
        200,
        "https://example.com/unknown".into(),
        Default::default(),
        b"<html></html>".to_vec(),
        String::new(),
        Request::get("https://example.com/unknown").with_callback("unknown"),
    );
    let (items, _) = spider.handle(resp_unknown).await;
    assert_eq!(items[0]["handler"], "default");
}

#[tokio::test]
async fn test_closure_spider_handle_default_handler() {
    // 无 callback 时，handle() 路由到 "default" handler
    let spider = SpiderBuilder::new("fallback")
        .start_urls(vec!["https://example.com/"])
        .on("default", |_resp| async move {
            (vec![json!({ "via": "default" })], vec![])
        })
        .build();

    let resp = Response::from_http(
        200,
        "https://example.com/".into(),
        Default::default(),
        b"<html></html>".to_vec(),
        String::new(),
        Request::get("https://example.com/"),
    );
    let (items, _) = spider.handle(resp).await;
    assert_eq!(items[0]["via"], "default");
}
