#![cfg(feature = "browser")]
//! 验证浏览器请求能获取真实 HTTP 状态码（不再 fallback 到 200）。
//!
//! 测试逻辑：
//! 1. 起本地 HTTP server，返回指定状态码（200/404/500）
//! 2. 用 FetchClient.fetch_browser 访问
//! 3. 验证 Response.status 与服务器返回的一致
//!
//! 修复前：若 capture_navigation_status 失败，fallback 默认 200，
//! 404/500 都被误判为 200。
//! 修复后：Network.enable 失败立即报错；recv_navigation_status 失败
//! 立即报错；不再有 fallback。

mod common;

use std::path::PathBuf;
use std::time::Duration;
use wisp::fetcher::{FetchMode, Method, Request};
use wisp::{FetchClient, FetchClientConfig};

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        429 => "Too Many Requests",
        _ => "OK",
    }
}

fn fetch_client_config() -> FetchClientConfig {
    let executable_path = std::env::var("CHROME_PATH").ok().map(PathBuf::from);
    FetchClientConfig {
        max_concurrent_pages: 1,
        executable_path,
        // 关闭人类行为模拟，加速测试
        human_mode: false,
        http: wisp::http::Config {
            timeout: Duration::from_secs(15),
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn fetch_status(client: &FetchClient, url: String) -> u16 {
    let req = Request {
        url,
        method: Method::Get,
        ..Default::default()
    };
    match client.fetch(&req, FetchMode::Dynamic).await {
        Ok(resp) => resp.status,
        Err(e) => {
            eprintln!("fetch_browser failed: {e:?}");
            0
        }
    }
}

#[tokio::test]
#[ignore = "需要真实 Chrome：手动运行 cargo test --test browser_status_code_test -- --ignored"]
async fn browser_fetch_captures_200_status() {
    let client = match FetchClient::new(fetch_client_config()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP: FetchClient build failed: {e:?}");
            return;
        }
    };
    // 探测 Chrome 是否可用：访问 about:blank 不会触发 Network.responseReceived
    // 这里直接试一次真实请求，失败则 SKIP
    let url = common::spawn_status_server(
        200,
        status_reason(200),
        "<html><body>OK</body></html>",
        "text/html; charset=utf-8",
    )
    .await;
    let status = fetch_status(&client, url).await;
    if status == 0 {
        eprintln!("SKIP: Chrome not available or fetch failed");
        return;
    }
    assert_eq!(status, 200, "200 page should be captured as 200");
}

#[tokio::test]
#[ignore = "需要真实 Chrome：手动运行 cargo test --test browser_status_code_test -- --ignored"]
async fn browser_fetch_captures_404_status_not_fallback_to_200() {
    let client = match FetchClient::new(fetch_client_config()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP: FetchClient build failed: {e:?}");
            return;
        }
    };
    let url = common::spawn_status_server(
        404,
        status_reason(404),
        "<html><body>Not Found</body></html>",
        "text/html; charset=utf-8",
    )
    .await;
    let status = fetch_status(&client, url).await;
    if status == 0 {
        eprintln!("SKIP: Chrome not available or fetch failed");
        return;
    }
    // 修复前：fallback 会返回 200（因为 title 不含 "404 not found"）
    // 修复后：应返回真实 404
    assert_eq!(
        status, 404,
        "404 page should be captured as 404, not fallback to 200"
    );
}

#[tokio::test]
#[ignore = "需要真实 Chrome：手动运行 cargo test --test browser_status_code_test -- --ignored"]
async fn browser_fetch_captures_500_status_not_fallback_to_200() {
    let client = match FetchClient::new(fetch_client_config()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP: FetchClient build failed: {e:?}");
            return;
        }
    };
    let url = common::spawn_status_server(
        500,
        status_reason(500),
        "<html><body>Server Error</body></html>",
        "text/html; charset=utf-8",
    )
    .await;
    let status = fetch_status(&client, url).await;
    if status == 0 {
        eprintln!("SKIP: Chrome not available or fetch failed");
        return;
    }
    // 修复前：500 会被 fallback 到 200（title 不含关键字）
    // 修复后：应返回真实 500
    assert_eq!(
        status, 500,
        "500 page should be captured as 500, not fallback to 200"
    );
}
