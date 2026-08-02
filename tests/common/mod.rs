//! Shared helpers for integration tests.

/// Start a local HTTP server returning one fixed status and body.
pub async fn spawn_status_server(
    status: u16,
    reason: &'static str,
    body: &'static str,
    content_type: &'static str,
) -> String {
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
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}

/// Start a local HTTP server returning a fixed HTML page with status 200.
#[allow(dead_code)]
pub async fn spawn_html_server(html: &'static str) -> String {
    spawn_status_server(200, "OK", html, "text/html; charset=utf-8").await
}
