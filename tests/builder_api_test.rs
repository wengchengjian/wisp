//! Builder pattern API tests (no network required).

use std::time::Duration;
use serde_json::{json, Value};
use wisp::crawl::{
    Spider, SpiderBuilder, Engine, Request, Response,
};
use wisp::crawl::CrawlEvent;
use wisp::parser::Node;
use futures::StreamExt;

// === SpiderBuilder tests ===

#[test]
fn test_spider_builder_full_config() {
    let spider = SpiderBuilder::new("full-test")
        .start_urls(vec!["https://a.com/", "https://b.com/"])
        .allowed_domains(vec!["a.com", "b.com"])
        .delay(Duration::from_millis(500))
        .obey_robots(false)
        .max_retries(5)
        .on("default", |_resp| async move {
            (vec![json!({"ok": true})], vec![])
        })
        .build();

    assert_eq!(spider.name(), "full-test");
    assert_eq!(spider.start_urls().len(), 2);
    assert_eq!(spider.download_delay(), Duration::from_millis(500));
    assert!(!spider.obey_robots());
    assert_eq!(spider.max_retries(), 5);
}

#[test]
fn test_spider_builder_delay_ms() {
    let spider = SpiderBuilder::new("delay-test")
        .start_urls(vec!["https://x.com/"])
        .delay_ms(250)
        .on("default", |_| async move { (vec![], vec![]) })
        .build();
    assert_eq!(spider.download_delay(), Duration::from_millis(250));
}

#[tokio::test]
async fn test_spider_builder_parse_with_follow() {
    let spider = SpiderBuilder::new("follow-test")
        .start_urls(vec!["https://example.com/"])
        .on("default", |resp| async move {
            let doc = resp.parse();
            let items: Vec<Value> = doc.select("h1").text().into_iter()
                .map(|t| json!({"title": t}))
                .collect();
            let follows = vec![Request::get("https://example.com/page2")];
            (items, follows)
        })
        .build();

    let resp = Response {
        url: "https://example.com/".into(),
        status: 200,
        headers: Default::default(),
        body: b"<html><body><h1>Home</h1></body></html>".to_vec(),
        request: Request::get("https://example.com/"),
        title: None,
        cookies: Vec::new(),
        content_type: String::new(),
        from_cache: false,
    };

    let (items, follows) = spider.handle(resp).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Home");
    assert_eq!(follows.len(), 1);
}

// === Response.follow() tests ===

#[test]
fn test_response_follow_absolute_url() {
    let resp = Response {
        url: "https://example.com/page1".into(),
        status: 200,
        headers: Default::default(),
        body: vec![],
        request: Request::get("https://example.com/page1"),
        title: None,
        cookies: Vec::new(),
        content_type: String::new(),
        from_cache: false,
    };
    let req = resp.follow("https://other.com/page2").unwrap();
    assert_eq!(req.url, "https://other.com/page2");
}

#[test]
fn test_response_follow_relative_path() {
    let resp = Response {
        url: "https://example.com/dir/page1".into(),
        status: 200,
        headers: Default::default(),
        body: vec![],
        request: Request::get("https://example.com/dir/page1"),
        title: None,
        cookies: Vec::new(),
        content_type: String::new(),
        from_cache: false,
    };
    let req = resp.follow("/page2").unwrap();
    assert_eq!(req.url, "https://example.com/page2");
}

#[test]
fn test_response_follow_with_callback() {
    let resp = Response {
        url: "https://example.com/".into(),
        status: 200,
        headers: Default::default(),
        body: vec![],
        request: Request::get("https://example.com/"),
        title: None,
        cookies: Vec::new(),
        content_type: String::new(),
        from_cache: false,
    };
    let req = resp.follow_with("/detail", "parse_detail").unwrap();
    assert_eq!(req.url, "https://example.com/detail");
    assert_eq!(req.callback, Some("parse_detail".to_string()));
}

// === Engine::infra() test ===

#[tokio::test]
async fn test_engine_builder_local_server() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let html = "<html><body><h1>Builder Test</h1></body></html>";
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let html = html;
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(), html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });

    let base_url = format!("http://{}", addr);

    let spider = SpiderBuilder::new("builder-test")
        .start_urls(vec![base_url])
        .obey_robots(false)
        .on("default", |resp| async move {
            let doc = resp.parse();
            let title = doc.select_one("h1").map(|n| n.text()).unwrap_or_default();
            (vec![json!({"title": title})], vec![])
        })
        .build();

    let engine = Engine::infra()
        .max_pages(1)
        .max_concurrent(2)
        .build()
        .unwrap();
    let (stats, _items) = engine.run(spider).await.unwrap();

    assert_eq!(stats.pages_crawled, 1);
    assert_eq!(stats.items_scraped, 1);
}

// === Node.find_by_text / find_similar tests ===

#[test]
fn test_find_by_text_exact() {
    let doc = Node::from_html(r#"<html><body>
        <div class="item">Apple</div>
        <div class="item">Banana</div>
        <div class="item">Apple Pie</div>
    </body></html>"#);

    let exact = doc.find_by_text("Apple", Some("div"), true);
    assert_eq!(exact.len(), 1);

    let contains = doc.find_by_text("Apple", Some("div"), false);
    assert_eq!(contains.len(), 2);
}

#[test]
fn test_find_similar_basic() {
    let doc = Node::from_html(r#"<html><body>
        <ul>
            <li class="item">First</li>
            <li class="item">Second</li>
            <li class="item">Third</li>
        </ul>
    </body></html>"#);

    let first_item = doc.select_one("li.item").unwrap();
    let similar = first_item.find_similar();
    assert!(similar.len() >= 2);
}

// === Stream + Builder test ===

#[tokio::test]
async fn test_stream_with_builder() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let html = "<html><body><p>Stream Item</p></body></html>";
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let html = html;
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(), html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });

    let base_url = format!("http://{}", addr);

    let spider = SpiderBuilder::new("stream-builder")
        .start_urls(vec![base_url])
        .obey_robots(false)
        .on("default", |resp| async move {
            let doc = resp.parse();
            let text = doc.select_one("p").map(|n| n.text()).unwrap_or_default();
            (vec![json!({"text": text})], vec![])
        })
        .build();

    let engine = Engine::infra().max_pages(1).build().unwrap();
    let mut stream = engine.run_stream(spider).events();

    let mut items = 0;
    let mut done = false;
    while let Some(event) = stream.next().await {
        match event {
            CrawlEvent::Item(_) => items += 1,
            CrawlEvent::Done(_) => { done = true; break; }
            _ => {}
        }
    }

    assert!(done);
    assert!(items >= 1);
}
