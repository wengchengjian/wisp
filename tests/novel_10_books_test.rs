use std::collections::HashSet;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wisp::crawl::stop::MaxPages;
use wisp::crawl::{Engine, Item, SpiderBuilder, on_page};
use wisp::fetcher::FetchMode;

async fn spawn_novel_server() -> String {
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
                let inner = if path == "/" {
                    (1..=12)
                        .map(|i| format!(r#"<a class="book" href="/book/{i}">Book {i}</a>"#))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else if path.starts_with("/book/") {
                    let book = path.trim_start_matches("/book/").to_string();
                    format!(
                        r#"<a class="chapter" href="/chapter/{book}/1">Ch1</a><a class="chapter" href="/chapter/{book}/2">Ch2</a>"#
                    )
                } else {
                    "<p>content</p>".to_string()
                };
                let body = format!("<html><body>{inner}</body></html>");
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

fn engine() -> Engine {
    Engine::infra()
        .max_concurrent(1)
        .max_pages(100)
        .obey_robots(false)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap()
}

fn assert_ten_books(items: &[Item]) {
    let books: HashSet<&str> = items
        .iter()
        .filter_map(|v| v.value()["title"].as_str())
        .collect();
    assert_eq!(books.len(), 10, "应爬取 10 本书，实际 {}", books.len());
    assert!(
        items.len() >= 20,
        "每本书应至少产出 2 个章节，实际 {}",
        items.len()
    );
}

#[tokio::test]
async fn handler_mode_crawls_ten_books() {
    let base = spawn_novel_server().await;
    let spider = SpiderBuilder::new("handler")
        .start_urls(vec![base.clone()])
        .on_page(
            "default",
            on_page!(page, {
                page.follow_links_n(
                    &["a.book"],
                    "detail",
                    10,
                    |_page, _i, a| serde_json::json!({ "title": a.text().trim() }),
                )
                .await;
                page
            }),
        )
        .on_page(
            "detail",
            on_page!(page, {
                let title = page.meta_str("title");
                page.follow_links(&["a.chapter"], "chapter", move |_page, _i, a| {
                    serde_json::json!({
                        "title": title,
                        "chapter_title": a.text().trim(),
                    })
                })
                .await;
                page
            }),
        )
        .on_page(
            "chapter",
            on_page!(page, {
                page.item(serde_json::json!({ "title": page.meta_str("title") }));
                page
            }),
        )
        .until(MaxPages(100))
        .build();
    let (_, items) = engine().run(spider).await.unwrap();
    assert_ten_books(&items);
}

#[tokio::test]
async fn spider_mode_crawls_ten_books() {
    let base = spawn_novel_server().await;
    let home = SpiderBuilder::new("home")
        .start_urls(vec![base.clone()])
        .on_page(
            "default",
            on_page!(page, {
                page.follow_links(
                    &["a.book"],
                    "detail",
                    |_page, _i, a| serde_json::json!({ "title": a.text().trim() }),
                )
                .await;
                page
            }),
        )
        .build();
    let detail = SpiderBuilder::new("detail")
        .on_page(
            "detail",
            on_page!(page, {
                let title = page.meta_str("title");
                page.follow_links(&["a.chapter"], "chapter", move |_page, _i, a| {
                    serde_json::json!({
                        "title": title,
                        "chapter_title": a.text().trim(),
                    })
                })
                .await;
                page
            }),
        )
        .until(MaxPages(10))
        .build();
    let chapter = SpiderBuilder::new("chapter")
        .on_page(
            "chapter",
            on_page!(page, {
                page.item(serde_json::json!({ "title": page.meta_str("title") }));
                page
            }),
        )
        .build();
    let (_, items) = engine()
        .run_many(vec![home, detail, chapter])
        .await
        .unwrap();
    assert_ten_books(&items);
}
