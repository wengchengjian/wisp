#![cfg(feature = "stealth")]
//! Regression: Turnstile solver can locate and click a cf-chl-widget iframe.
//!
//! This uses a local same-origin iframe fixture so the test is deterministic.
//! Clicking inside the widget changes the parent title, which the solver uses
//! as its bypass signal.

use wisp::fetcher::Fetcher;
use wisp::stealth::TurnstileConfig;

async fn spawn_turnstile_fixture() -> String {
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
                let (body, status) = if path == "/widget" {
                    (
                        "<html><body style=\"margin:0;width:100%;height:100%\"><div style=\"width:60px;height:60px\" onclick=\"parent.document.title='passed'\">x</div></body></html>",
                        "200 OK",
                    )
                } else {
                    (
                        "<html><head><title>Just a moment</title></head><body><iframe id=\"cf-chl-widget-test\" src=\"/widget\" style=\"width:300px;height:65px\"></iframe></body></html>",
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
#[ignore = "需要真实 Chrome：手动运行 cargo test --test turnstile_local_regression_test -- --ignored"]
async fn turnstile_solver_clicks_local_widget() {
    let base = spawn_turnstile_fixture().await;

    let config = TurnstileConfig {
        passive_wait_ms: 100,
        click_interval_ms: 100,
        poll_interval_ms: 50,
        mouse_steps: 1,
        mouse_step_delay_ms: 1,
        click_hold_ms: 20,
        dom_depth: 10,
        human_mode: false,
    };
    let resp = Fetcher::stealth()
        .headless(true)
        .human_mode(false)
        .turnstile_config(config)
        .get(&base)
        .await
        .expect("solver should use headed+offscreen rendering and click the local widget");
    assert_eq!(resp.status, 200);
    assert_eq!(
        resp.title(),
        Some("passed"),
        "solver should update the parent title: {:?}",
        resp.title()
    );
}
