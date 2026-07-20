use chromiumoxide::Page as CdpPage;

use crate::error::{PatchrightError, Result};

/// Navigate the page to a URL and wait for load.
///
/// chromiumoxide's `goto` already waits for the page to fully load.
pub async fn goto(page: &CdpPage, url: &str) -> Result<()> {
    page.goto(url)
        .await
        .map_err(|e| PatchrightError::NavigationFailed(e.to_string()))?;
    Ok(())
}

/// Reload the current page.
///
/// chromiumoxide's `reload` already waits for navigation.
pub async fn reload(page: &CdpPage) -> Result<()> {
    page.reload()
        .await
        .map_err(|e| PatchrightError::NavigationFailed(e.to_string()))?;
    Ok(())
}
