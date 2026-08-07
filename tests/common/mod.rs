//! Shared helpers for integration tests.
#![expect(
    dead_code,
    reason = "shared test helper may be unused per integration binary"
)]

async fn spawn_status_server_on(
    url_host: &str,
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
    format!("http://{url_host}:{}", addr.port())
}

/// Start a local HTTP server returning one fixed status and body.
pub async fn spawn_status_server(
    status: u16,
    reason: &'static str,
    body: &'static str,
    content_type: &'static str,
) -> String {
    spawn_status_server_on("127.0.0.1", status, reason, body, content_type).await
}

/// Start a local HTTP server returning a fixed HTML page with status 200.
pub async fn spawn_html_server(html: &'static str) -> String {
    spawn_status_server(200, "OK", html, "text/html; charset=utf-8").await
}

/// Same as [`spawn_html_server`] but exposes the URL through `localhost` so
/// MCP SSRF checks can resolve it to the local test listener.
pub async fn spawn_localhost_html_server(html: &'static str) -> String {
    spawn_status_server_on("localhost", 200, "OK", html, "text/html; charset=utf-8").await
}

/// Start a server that redirects every request to `location`.
pub async fn spawn_redirect_server(location: String) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let location = location.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}/start")
}

/// Start a server that returns a body of the requested size.
pub async fn spawn_large_body_server(size: usize) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let body = "x".repeat(size);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}
