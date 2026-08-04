use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wisp::crawl::Engine;
use wisp::crawl::stop::MaxPages;
use wisp::crawl::{CrawlRequest, Spider, SpiderBuilder};
use wisp::fetcher::FetchMode;

#[test]
fn closure_spider_accepts_only_owned_callbacks() {
    let spider = SpiderBuilder::new("detail")
        .on_page("detail", |page| page)
        .build();

    assert!(!spider.accepts_callback(None));
    assert!(spider.accepts_callback(Some("detail")));
    assert!(!spider.accepts_callback(Some("chapter")));

    let home = SpiderBuilder::new("home")
        .on_page("default", |page| page)
        .build();
    assert!(home.accepts_callback(None));
    assert!(!home.accepts_callback(Some("unknown")));

    let req = CrawlRequest::get("https://example.com/book/1").with_callback("detail");
    assert!(spider.accepts_callback(req.callback.as_deref()));
}

async fn spawn_stage_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let request_line = String::from_utf8_lossy(&buf);
                let path = request_line
                    .lines()
                    .next()
                    .unwrap_or("GET / HTTP/1.1")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/");
                let body = if path == "/" {
                    "<html><body><a href=\"/book/1\">1</a><a href=\"/book/2\">2</a><a href=\"/book/3\">3</a></body></html>".to_string()
                } else {
                    let book = path.trim_start_matches("/book/").to_string();
                    format!("<html><body><a href=\"/chapter/{book}\">Ch</a></body></html>")
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{}", addr)
}

#[tokio::test]
async fn detail_spider_until_does_not_block_chapter_spider() {
    let base = spawn_stage_server().await;
    let home = SpiderBuilder::new("home")
        .start_urls(vec![base.clone()])
        .on_page("default", |mut page| {
            page.follow_links(
                &["a"],
                "detail",
                |_page, _i, a| serde_json::json!({ "title": a.text().trim() }),
            );
            page
        })
        .build();
    let detail = SpiderBuilder::new("detail")
        .on_page("detail", |mut page| {
            let title = page.meta_str("title");
            page.follow_links(
                &["a"],
                "chapter",
                |_page, _i, _a| serde_json::json!({ "title": title.clone() }),
            );
            page
        })
        .until(MaxPages(2))
        .build();
    let chapter = SpiderBuilder::new("chapter")
        .on_page("chapter", |mut page| {
            page.item(serde_json::json!({ "title": page.meta_str("title") }));
            page
        })
        .build();

    let engine = Engine::infra()
        .max_concurrent(1)
        .max_pages(100)
        .obey_robots(false)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();

    let (stats, items) = engine.run_many(vec![home, detail, chapter]).await.unwrap();
    assert_eq!(stats[0].pages_crawled, 1, "home 只爬首页");
    assert_eq!(stats[1].pages_crawled, 2, "detail 只爬 2 个详情");
    assert_eq!(items.len(), 2, "chapter 应产出 2 个 item");
}
