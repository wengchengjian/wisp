use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page as CdpPage;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;

use crate::error::{PatchrightError, Result};

/// Capture a screenshot and save to file as PNG.
pub async fn screenshot(page: &CdpPage, path: &str) -> Result<()> {
    let bytes = screenshot_bytes(page).await?;
    std::fs::write(std::path::Path::new(path), &bytes)?;
    Ok(())
}

/// Capture a screenshot and return raw PNG bytes.
pub async fn screenshot_bytes(page: &CdpPage) -> Result<Vec<u8>> {
    let params = ScreenshotParams::builder()
        .format(CaptureScreenshotFormat::Png)
        .full_page(true)
        .build();

    page.screenshot(params)
        .await
        .map_err(|e| PatchrightError::CdpError(format!("Screenshot failed: {e}")))
}
