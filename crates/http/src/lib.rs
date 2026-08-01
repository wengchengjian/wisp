//! HTTP client with automatic encoding detection.
//!
//! Wraps wreq with builder pattern, proxy support, and HTML parsing.

pub mod block;
mod builder;
mod client;
mod config;
mod dns;

pub use block::DomainBlocker;
pub use builder::ClientBuilder;
pub use client::Client;
pub use config::Config;
pub use dns::DoHResolver;

#[cfg(test)]
mod tests {
    use super::*;

    /// 启动测试 HTTP 服务器，回显收到的请求 headers（每行一个 header）。
    async fn spawn_echo_server() -> String {
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
                    let mut buf = vec![0u8; 16384];
                    let mut total = 0usize;
                    // 循环读取直到收到完整的 HTTP 请求头（\r\n\r\n 结尾）
                    while total < buf.len() {
                        let n = socket.read(&mut buf[total..]).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        total += n;
                        if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8_lossy(&buf[..total]);
                    // 回显收到的 headers（跳过请求行）
                    let headers: String = request
                        .lines()
                        .skip(1)
                        .take_while(|line| !line.is_empty())
                        .filter(|line| line.contains(':'))
                        .map(|line| format!("{}\n", line))
                        .collect();
                    let body = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        headers.len(),
                        headers
                    );
                    let _ = socket.write_all(body.as_bytes()).await;
                });
            }
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn put_with_extra_headers_sends_them() {
        let base = spawn_echo_server().await;
        let client = Client::builder().no_emulation().build().unwrap();
        let extra = vec![("X-Custom".to_string(), "put-val".to_string())];
        let resp = client
            .put(&format!("{}/item", base), None, None, &extra)
            .await
            .unwrap();
        let text = resp.text().unwrap();
        assert!(
            text.to_ascii_lowercase().contains("x-custom: put-val"),
            "PUT 应发送 extra headers, 实际: {text}"
        );
    }

    #[tokio::test]
    async fn delete_with_extra_headers_sends_them() {
        let base = spawn_echo_server().await;
        let client = Client::builder().no_emulation().build().unwrap();
        let extra = vec![("X-Custom".to_string(), "del-val".to_string())];
        let resp = client
            .delete(&format!("{}/item", base), &extra)
            .await
            .unwrap();
        let text = resp.text().unwrap();
        assert!(
            text.to_ascii_lowercase().contains("x-custom: del-val"),
            "DELETE 应发送 extra headers, 实际: {text}"
        );
    }

    #[tokio::test]
    async fn get_with_empty_extra_headers_still_works() {
        let base = spawn_echo_server().await;
        let client = Client::builder().no_emulation().build().unwrap();
        let resp = client.get(&format!("{}/item", base), &[]).await.unwrap();
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn socks5_proxy_builds() {
        let client = Client::builder().proxy("socks5://127.0.0.1:1080").build();
        assert!(client.is_ok(), "socks5 应可构建: {:?}", client.err());
    }

    #[test]
    fn socks4_proxy_builds() {
        let client = Client::builder().proxy("socks4://127.0.0.1:1080").build();
        assert!(client.is_ok(), "socks4 应可构建: {:?}", client.err());
    }
}
