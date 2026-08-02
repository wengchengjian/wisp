//! Regression: blocked responses must not count as crawled pages.
//!
//! `process_response` used to increment `pages_crawled` before response
//! middleware ran. When `BlockedRetryMiddleware` exhausted its refetch rounds,
//! the response was discarded but the page counter stayed incremented.

use wisp::crawl::{Engine, SpiderBuilder};
use wisp::FetchMode;

async fn spawn_403_server() -> String {
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
                let body = "blocked";
                let response = format!(
                    "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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
async fn blocked_response_should_not_increment_pages_after_refetch_exhausted() {
    let base = spawn_403_server().await;
    let spider = SpiderBuilder::new("blocked")
        .start_urls(vec![base.clone()])
        .on("default", |_resp| async move { (vec![], vec![]) })
        .build();

    let engine = Engine::infra()
        .fetch_mode(FetchMode::Http)
        .max_pages(1)
        .obey_robots(false)
        .max_refetch_rounds(2)
        .build()
        .unwrap();
    let (stats, _items) = engine.run(spider).await.unwrap();

    assert_eq!(
        stats.pages_crawled,
        0,
        "blocked response should not count as a crawled page, stats: {}",
        stats.summary()
    );
    assert!(
        stats.blocked_requests >= 3,
        "initial 403 plus refetches should be counted as blocked, stats: {}",
        stats.summary()
    );
}
