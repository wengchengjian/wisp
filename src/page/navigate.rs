use std::sync::Arc;
use serde_json::json;
use crate::cdp::session::CdpSession;
use crate::error::{PatchrightError, Result};

pub async fn goto(session: &Arc<CdpSession>, url: &str) -> Result<()> {
    let result = session.execute("Page.navigate", json!({ "url": url })).await?;
    if let Some(error_text) = result.get("errorText").and_then(|e| e.as_str()) {
        if !error_text.is_empty() {
            return Err(PatchrightError::NavigationFailed(error_text.to_string()));
        }
    }
    // Wait for load event
    wait_for_load(session).await?;
    Ok(())
}

pub async fn reload(session: &Arc<CdpSession>) -> Result<()> {
    session.execute("Page.reload", json!({})).await?;
    wait_for_load(session).await?;
    Ok(())
}

async fn wait_for_load(session: &Arc<CdpSession>) -> Result<()> {
    let mut events = session.events();
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
