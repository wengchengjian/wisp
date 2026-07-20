use serde_json::json;
use base64::Engine;
use crate::error::{PatchrightError, Result};
use super::Page;

pub async fn screenshot(page: &Page, path: &str) -> Result<()> {
    let bytes = screenshot_bytes(page).await?;
    std::fs::write(std::path::Path::new(path), &bytes)?;
    Ok(())
}

pub async fn screenshot_bytes(page: &Page) -> Result<Vec<u8>> {
    let result = page.cmd("Page.captureScreenshot", json!({
        "format": "png"
    })).await?;

    let data = result.get("data")
        .and_then(|d| d.as_str())
        .ok_or_else(|| PatchrightError::CdpError("no screenshot data".into()))?;

    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| PatchrightError::CdpError(format!("base64 decode: {e}")))
}
