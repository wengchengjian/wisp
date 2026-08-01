use super::*;
use wisp_core::config::LaunchOptions;

// === 集成测试（需要 Chrome 环境） ===
// 运行方式：cargo test --lib browser::tests -- --ignored

/// 测试浏览器启动和关闭。
#[tokio::test]
#[ignore = "需要 Chrome 浏览器环境"]
async fn test_browser_launch_and_close() {
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .expect("浏览器应成功启动");

    browser.close().await.expect("浏览器应成功关闭");
}

/// 测试创建新页面（tab）。
#[tokio::test]
#[ignore = "需要 Chrome 浏览器环境"]
async fn test_browser_new_page() {
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .expect("启动浏览器");

    let mut page = browser.new_page().await.expect("应成功创建页面");
    assert!(!page.session_id().is_empty(), "页面应有 session_id");

    let _ = page.close().await;
    browser.close().await.expect("关闭浏览器");
}

/// 测试页面导航和 JS 执行。
#[tokio::test]
#[ignore = "需要 Chrome 浏览器环境"]
async fn test_page_navigation_and_evaluate() {
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .expect("启动浏览器");

    let mut page = browser.new_page().await.expect("创建页面");

    // 导航到 data URL
    page.goto("data:text/html,<html><body><h1>Test</h1></body></html>")
        .await
        .expect("导航应成功");

    // 执行 JS
    let result = page.evaluate("1 + 1").await.expect("JS 执行应成功");
    assert_eq!(result.as_i64(), Some(2));

    // 获取字符串
    let text = page
        .evaluate_as_string("document.querySelector('h1').textContent")
        .await
        .expect("获取文本应成功");
    assert_eq!(text, "Test");

    let _ = page.close().await;
    browser.close().await.expect("关闭浏览器");
}

/// 测试 CDP 命令执行。
#[tokio::test]
#[ignore = "需要 Chrome 浏览器环境"]
async fn test_cdp_command() {
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .expect("启动浏览器");

    let mut page = browser.new_page().await.expect("创建页面");

    // 执行 CDP 命令获取浏览器版本
    let result = page.cmd("Browser.getVersion", json!({})).await;
    assert!(result.is_ok(), "Browser.getVersion 应成功");
    let version_info = result.unwrap();
    assert!(version_info.get("product").is_some(), "应包含 product 字段");

    let _ = page.close().await;
    browser.close().await.expect("关闭浏览器");
}

/// 测试多页面并发。
#[tokio::test]
#[ignore = "需要 Chrome 浏览器环境"]
async fn test_multiple_pages() {
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .expect("启动浏览器");

    let mut page1 = browser.new_page().await.expect("创建页面 1");
    let mut page2 = browser.new_page().await.expect("创建页面 2");

    // 两个页面应有不同的 session_id
    assert_ne!(
        page1.session_id(),
        page2.session_id(),
        "不同页面应有不同 session_id"
    );

    let _ = page1.close().await;
    let _ = page2.close().await;
    browser.close().await.expect("关闭浏览器");
}
