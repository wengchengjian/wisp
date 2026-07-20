use serde_json::json;
use base64::Engine;
use crate::error::{WispError, Result};
use super::Page;

pub async fn screenshot(page: &Page, path: &str) -> Result<()> {
    let bytes = screenshot_bytes(page).await?;
    tokio::fs::write(path, &bytes).await
        .map_err(|e| WispError::CdpError(format!("write: {e}")))?;
    Ok(())
}

pub async fn screenshot_bytes(page: &Page) -> Result<Vec<u8>> {
    let result = page.cmd("Page.captureScreenshot", json!({"format": "png"})).await?;
    let data = result.get("data").and_then(|d| d.as_str())
        .ok_or_else(|| WispError::CdpError("no screenshot data".into()))?;
    base64::engine::general_purpose::STANDARD.decode(data)
        .map_err(|e| WispError::CdpError(format!("decode: {e}")))
}
