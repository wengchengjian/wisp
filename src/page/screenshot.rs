use std::sync::Arc;
use serde_json::json;
use base64::Engine;
use crate::cdp::session::CdpSession;
use crate::error::{PatchrightError, Result};

pub async fn screenshot(session: &Arc<CdpSession>, path: &str) -> Result<()> {
    let bytes = screenshot_bytes(session).await?;
    std::fs::write(std::path::Path::new(path), &bytes)?;
    Ok(())
}

pub async fn screenshot_bytes(session: &Arc<CdpSession>) -> Result<Vec<u8>> {
    let result = session.execute("Page.captureScreenshot", json!({
        "format": "png"
    })).await?;

    let data = result.get("data")
        .and_then(|d| d.as_str())
        .ok_or_else(|| PatchrightError::CdpError("no screenshot data".into()))?;

    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| PatchrightError::CdpError(format!("base64 decode: {e}")))
}
