//! Builder pattern API tests (no network required).

mod common;

use futures::StreamExt;
use serde_json::{Value, json};
use wisp::crawl::CrawlEvent;
use wisp::crawl::{CrawlRequest, Engine, Request, Response, Spider, SpiderBuilder};
use wisp::parser::Node;
use wisp::parser::ResponseExt;

// === SpiderBuilder tests ===

#[test]
fn test_spider_builder_full_config() {
    let spider = SpiderBuilder::new("full-test")
        .start_urls(vec!["https://a.com/", "https://b.com/"])
        .allowed_domains(vec!["a.com", "b.com"])
        .on("default", |_resp| async move { (vec![json!({"ok": true})], vec![]) })
        .build();

    assert_eq!(spider.name(), "full-test");
    assert_eq!(spider.start_urls().len(), 2);
}

#[tokio::test]
async fn test_spider_builder_parse_with_follow() {
    let spider = SpiderBuilder::new("follow-test")
        .start_urls(vec!["https://example.com/"])
        .on("default", |resp| async move {
            let doc = resp.parse();
            let items: Vec<Value> = doc
                .select("h1")
                .text()
                .into_iter()
                .map(|t| json!({"title": t}))
                .collect();
            let follows = vec![CrawlRequest::get("https://example.com/page2")];
            (items, follows)
        })
        .build();

    let resp = Response::from_http(
        200,
        "https://example.com/".into(),
        Default::default(),
        b"<html><body><h1>Home</h1></body></html>".to_vec(),
        String::new(),
        Request::get("https://example.com/"),
    );

    let (items, follows) = spider.handle(resp).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Home");
    assert_eq!(follows.len(), 1);
}

// === Response.follow() tests ===

#[test]
fn test_response_follow_absolute_url() {
    let resp = Response::from_http(
        200,
        "https://example.com/page1".into(),
        Default::default(),
        vec![],
        String::new(),
        Request::get("https://example.com/page1"),
    );
    let req = resp.follow("https://other.com/page2").unwrap();
    assert_eq!(req.url, "https://other.com/page2");
}

#[test]
fn test_response_follow_relative_path() {
    let resp = Response::from_http(
        200,
        "https://example.com/dir/page1".into(),
        Default::default(),
        vec![],
        String::new(),
        Request::get("https://example.com/dir/page1"),
    );
    let req = resp.follow("/page2").unwrap();
    assert_eq!(req.url, "https://example.com/page2");
}

#[test]
fn test_response_follow_with_callback() {
    let resp = Response::from_http(
        200,
        "https://example.com/".into(),
        Default::default(),
        vec![],
        String::new(),
        Request::get("https://example.com/"),
    );
    let req = resp.follow_with("/detail", "parse_detail").unwrap();
    assert_eq!(req.url, "https://example.com/detail");
    assert_eq!(req.callback, Some("parse_detail".to_string()));
}

// === Engine::infra() test ===

#[tokio::test]
async fn test_engine_builder_local_server() {
    let base_url =
        common::spawn_html_server("<html><body><h1>Builder Test</h1></body></html>").await;

    let spider = SpiderBuilder::new("builder-test")
        .start_urls(vec![base_url])
        .on("default", |resp| async move {
            let doc = resp.parse();
            let title = doc.select_one("h1").map(|n| n.text()).unwrap_or_default();
            (vec![json!({"title": title})], vec![])
        })
        .build();

    let engine = Engine::infra()
        .max_pages(1)
        .max_concurrent(2)
        .obey_robots(false)
        .build()
        .unwrap();
    let (stats, _items) = engine.run(spider).await.unwrap();

    assert_eq!(stats.pages_crawled, 1);
    assert_eq!(stats.items_scraped, 1);
}

// === Node.find_by_text / find_similar tests ===

#[test]
fn test_find_by_text_exact() {
    let doc = Node::from_html(
        r#"<html><body>
        <div class="item">Apple</div>
        <div class="item">Banana</div>
        <div class="item">Apple Pie</div>
    </body></html>"#,
    );

    let exact = doc.find_by_text("Apple", Some("div"), true);
    assert_eq!(exact.len(), 1);

    let contains = doc.find_by_text("Apple", Some("div"), false);
    assert_eq!(contains.len(), 2);
}

#[test]
fn test_find_similar_basic() {
    let doc = Node::from_html(
        r#"<html><body>
        <ul>
            <li class="item">First</li>
            <li class="item">Second</li>
            <li class="item">Third</li>
        </ul>
    </body></html>"#,
    );

    let first_item = doc.select_one("li.item").unwrap();
    let similar = first_item.find_similar();
    assert!(similar.len() >= 2);
}

// === Stream + Builder test ===

#[tokio::test]
async fn test_stream_with_builder() {
    let base_url = common::spawn_html_server("<html><body><p>Stream Item</p></body></html>").await;

    let spider = SpiderBuilder::new("stream-builder")
        .start_urls(vec![base_url])
        .on("default", |resp| async move {
            let doc = resp.parse();
            let text = doc.select_one("p").map(|n| n.text()).unwrap_or_default();
            (vec![json!({"text": text})], vec![])
        })
        .build();

    let engine = Engine::infra()
        .max_pages(1)
        .obey_robots(false)
        .build()
        .unwrap();
    let mut stream = engine.run_stream(spider).events();

    let mut items = 0;
    let mut done = false;
    while let Some(event) = stream.next().await {
        match event {
            CrawlEvent::Item(_) => items += 1,
            CrawlEvent::Done(_) => {
                done = true;
                break;
            }
            _ => {}
        }
    }

    assert!(done);
    assert!(items >= 1);
}

#[tokio::test]
async fn test_http_client_follows_redirect() {
    let target = common::spawn_html_server("<h1>Redirected</h1>").await;
    let start = common::spawn_redirect_server(target.clone()).await;
    let client = wisp::http::Client::builder().build().unwrap();
    let resp = client.get(&start, &[]).await.unwrap();
    assert_eq!(resp.status, 200);
    assert!(resp.url.starts_with(&target));
    assert!(resp.text().unwrap().contains("Redirected"));
}

#[tokio::test]
async fn test_http_client_rejects_oversized_body() {
    let base = common::spawn_large_body_server(1024).await;
    let client = wisp::http::Client::builder()
        .max_body_size(16)
        .build()
        .unwrap();
    let err = client.get(&base, &[]).await.unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
}
