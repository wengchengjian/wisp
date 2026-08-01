//! MCP stealth_fetch 工具。

#[cfg(feature = "browser")]
use serde_json::Value;
#[cfg(feature = "browser")]
use wisp_core::error::{McpError, Result, WispError};

/// 浏览器模式抓取（绕 CF Turnstile）。
#[cfg(feature = "browser")]
async fn run_stealth_fetch(url: &str, headless: bool, human_mode: bool) -> Result<Value> {
    use serde_json::json;
    use wisp_browser::Browser;
    use wisp_core::config::LaunchOptions;
    let browser = Browser::launch(LaunchOptions {
        headless,
        ..Default::default()
    })
    .await
    .map_err(|e| WispError::Mcp(McpError::General(format!("browser launch: {e}"))))?;
    let mut page = browser
        .new_page()
        .await
        .map_err(|e| WispError::Mcp(McpError::General(format!("new page: {e}"))))?;
    page.goto(url)
        .await
        .map_err(|e| WispError::Mcp(McpError::General(format!("goto: {e}"))))?;
    if human_mode {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let html = page
        .evaluate_as_string("document.documentElement.outerHTML")
        .await
        .map_err(|e| WispError::Mcp(McpError::General(format!("get html: {e}"))))?;
    let title = page
        .evaluate_as_string("document.title")
        .await
        .unwrap_or_default();
    browser
        .close()
        .await
        .map_err(|e| WispError::Mcp(McpError::General(format!("close: {e}"))))?;
    Ok(json!({
        "url": url,
        "title": title,
        "html": html,
        "bytes": html.len()
    }))
}

#[cfg(feature = "browser")]
pub async fn stealth_fetch(args: Value) -> Result<Value> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'url'".into())))?;
    let headless = args
        .get("headless")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let human_mode = args
        .get("human_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    run_stealth_fetch(url, headless, human_mode).await
}
