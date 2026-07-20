use serde_json::json;
use crate::error::{PatchrightError, Result};
use super::Page;

pub async fn goto(page: &Page, url: &str) -> Result<()> {
    let result = page.cmd("Page.navigate", json!({ "url": url })).await?;
    if let Some(error_text) = result.get("errorText").and_then(|e| e.as_str()) {
        if !error_text.is_empty() {
            return Err(PatchrightError::NavigationFailed(error_text.to_string()));
        }
    }
    // Wait for load event
    wait_for_load(page).await?;
    Ok(())
}

pub async fn reload(page: &Page) -> Result<()> {
    page.cmd("Page.reload", json!({})).await?;
    wait_for_load(page).await?;
    Ok(())
}

async fn wait_for_load(page: &Page) -> Result<()> {
    let mut events = page.session.events();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(event)) => {
                if event.method == "Page.loadEventFired" {
                    return Ok(());
                }
                // Also accept lifecycleEvent with name "load"
                if event.method == "Page.lifecycleEvent" {
                    if event.params.get("name").and_then(|n| n.as_str()) == Some("load") {
                        return Ok(());
                    }
                }
            }
            Ok(Err(_)) => continue,
            Err(_) => return Err(PatchrightError::Timeout("wait for page load".into())),
        }
    }
}
