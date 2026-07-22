//! Task 1 回归测试：crawl_site 传入 start_urls 修复。
use serde_json::json;
use wisp::mcp::tools::crawl_site;
use wisp::storage::Store;
use std::sync::Arc;

#[tokio::test]
async fn test_crawl_site_uses_start_urls() {
    let server = spawn_html_server("<p>item1</p><p>item2</p>").await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let args = json!({
        "start_urls": [server],
        "css_selector": "p",
        "max_pages": 1
    });
    let result = crawl_site(args, &store).await.expect("crawl_site should succeed");
    assert_eq!(result["items_count"].as_u64(), Some(2), "应爬到 2 个 p 元素");
}

async fn spawn_html_server(html: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(), html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{}", addr)
}
