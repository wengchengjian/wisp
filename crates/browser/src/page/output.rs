//! Screenshot and PDF output.

use super::*;
use base64::Engine;
use wisp_core::error::{BrowserError, WispError};

impl Page {
    /// 截图并保存到文件。
    pub async fn screenshot(&self, path: &str) -> Result<()> {
        do_screenshot(self, path).await
    }
    /// 截图并返回 PNG 字节数据。
    pub async fn screenshot_bytes(&self) -> Result<Vec<u8>> {
        do_screenshot_bytes(self).await
    }

    /// Generate a PDF (headless only).
    pub async fn pdf(&self, path: &str) -> Result<()> {
        let result = self
            .cmd("Page.printToPDF", json!({ "printBackground": true }))
            .await?;
        let data = result
            .get("data")
            .and_then(|d| d.as_str())
            .ok_or_else(|| WispError::Browser(BrowserError::Other("no PDF data".into())))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| WispError::Browser(BrowserError::Other(format!("decode PDF: {e}"))))?;
        tokio::fs::write(path, &bytes)
            .await
            .map_err(|e| WispError::Browser(BrowserError::Other(format!("write PDF: {e}"))))?;
        Ok(())
    }
}

/// 截图并保存到指定路径。
pub async fn do_screenshot(page: &Page, path: &str) -> Result<()> {
    let bytes = do_screenshot_bytes(page).await?;
    tokio::fs::write(path, &bytes)
        .await
        .map_err(|e| WispError::Browser(BrowserError::Other(format!("write: {e}"))))?;
    Ok(())
}

/// 截图并返回 PNG 格式的字节数据。
pub async fn do_screenshot_bytes(page: &Page) -> Result<Vec<u8>> {
    let result = page
        .cmd("Page.captureScreenshot", json!({ "format": "png" }))
        .await?;
    let data = result
        .get("data")
        .and_then(|d| d.as_str())
        .ok_or_else(|| WispError::Browser(BrowserError::Other("no screenshot data".into())))?;
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| WispError::Browser(BrowserError::Other(format!("decode: {e}"))))
}
