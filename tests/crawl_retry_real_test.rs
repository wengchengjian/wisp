//! 重试机制回归测试：使用本地确定性服务器验证 blocked refetch 语义。

use async_trait::async_trait;
use serde_json::Value;
use wisp::crawl::{Engine, Request, Response, Spider};
use wisp::FetchMode;

async fn spawn_status_server(status: u16, reason: &'static str, body: &'static str) -> String {
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
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}

struct RetrySpider {
    base: String,
}

#[async_trait]
impl Spider for RetrySpider {
    fn name(&self) -> &str {
        "retry-test"
    }
    fn start_urls(&self) -> Vec<String> {
        vec![self.base.clone()]
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
        (vec![], vec![])
    }
}

#[tokio::test]
async fn test_retry_on_403_status() {
    let base = spawn_status_server(403, "Forbidden", "blocked").await;
    let engine = Engine::infra()
        .max_pages(1)
        .obey_robots(false)
        .fetch_mode(FetchMode::Http)
        .max_retries(2)
        .max_refetch_rounds(2)
        .download_delay(std::time::Duration::from_millis(100))
        .build()
        .unwrap();
    let (stats, _items) = engine.run(RetrySpider { base }).await.unwrap();

    // 403 走 BlockedRetry refetch，不增加网络错误 retry_count/errors
    assert_eq!(stats.errors, 0, "403 不应计入网络错误: {:?}", stats);
    assert_eq!(stats.pages_crawled, 0, "403 不应计入成功页: {:?}", stats);
    assert_eq!(
        stats.retry_count, 0,
        "403 不应增加 retry_count: {:?}",
        stats
    );
    assert!(
        stats.blocked_requests >= 3,
        "403 应至少统计 3 次 blocked: {:?}",
        stats
    );
}

#[tokio::test]
async fn test_http_success_no_retry_count() {
    struct OkSpider {
        base: String,
    }
    #[async_trait]
    impl Spider for OkSpider {
        fn name(&self) -> &str {
            "ok-test"
        }
        fn start_urls(&self) -> Vec<String> {
            vec![self.base.clone()]
        }
        async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
            (vec![], vec![])
        }
    }

    let base = spawn_status_server(200, "OK", "ok").await;
    let engine = Engine::infra()
        .max_pages(1)
        .obey_robots(false)
        .fetch_mode(FetchMode::Http)
        .max_retries(2)
        .build()
        .unwrap();
    let (stats, _items) = engine.run(OkSpider { base }).await.unwrap();
    assert_eq!(stats.pages_crawled, 1);
    assert_eq!(stats.retry_count, 0);
}
