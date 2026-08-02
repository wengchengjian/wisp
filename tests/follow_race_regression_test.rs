//! Regression: Engine must consume follows produced by a single start URL.
//!
//! `run_work_loop` uses `unfold + buffer_unordered`; with one start URL the
//! buffered Work future is created before it starts running, so the next
//! scheduling pass can see an empty queue with `global_in_flight == 0` and
//! finish before the follow request is consumed.

use std::time::Duration;
use wisp::crawl::{Engine, SpiderBuilder};
use wisp::parser::ResponseExt;
use wisp::FetchMode;

async fn spawn_two_page_server() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let request = String::from_utf8_lossy(&buf);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let (body, status) = if path == "/page2" {
                    ("<html><body><h1>page2</h1></body></html>", "200 OK")
                } else {
                    (
                        "<html><body><div class=\"next\"><a href=\"/page2\">Next</a></div></body></html>",
                        "200 OK",
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn engine_should_consume_follow_produced_by_single_start_url() {
    let base = spawn_two_page_server().await;
    let spider = SpiderBuilder::new("follow-race")
        .start_urls(vec![base.clone()])
        .on("default", |resp| async move {
            let doc = resp.parse();
            let follows: Vec<wisp::Request> = doc
                .select_one(".next a")
                .and_then(|a| a.attr("href"))
                .and_then(|href| resp.follow(&href))
                .into_iter()
                .collect();
            (vec![], follows)
        })
        .build();

    let engine = Engine::infra()
        .fetch_mode(FetchMode::Http)
        .max_pages(2)
        .obey_robots(false)
        .download_delay(Duration::ZERO)
        .build()
        .unwrap();
    let (stats, _items) = engine.run(spider).await.unwrap();

    assert_eq!(
        stats.pages_crawled,
        2,
        "single start URL should crawl the followed page, stats: {}",
        stats.summary()
    );
}
