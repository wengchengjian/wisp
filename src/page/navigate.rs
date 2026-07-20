use serde_json::json;
use crate::error::Result;
use super::Page;

pub async fn goto(page: &Page, url: &str) -> Result<()> {
    page.cmd("Page.navigate", json!({ "url": url })).await?;
    // Wait for page load using lifecycle event or timeout
    wait_for_load(page).await
}

pub async fn reload(page: &Page) -> Result<()> {
    page.cmd("Page.reload", json!({})).await?;
    wait_for_load(page).await
}

async fn wait_for_load(page: &Page) -> Result<()> {
    // Try to wait for load event, but don't fail if it times out
    // (some pages like about:blank don't fire load events the same way)
    let sid = page.session_id.clone();
    let result = page.session.wait_for_event(
        move |e| {
            if e.method == "Page.loadEventFired" {
                return e.session_id.as_deref() == Some(sid.as_str()) || e.session_id.is_none();
            }
            // Also accept lifecycleEvent with name "load"
            if e.method == "Page.lifecycleEvent" {
                if e.params.get("name").and_then(|n| n.as_str()) == Some("load") {
                    return e.session_id.as_deref() == Some(sid.as_str()) || e.session_id.is_none();
                }
            }
            false
        },
        15000,
    ).await;

    // If event wait fails, fall back to a short delay
    if result.is_err() {
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }
    Ok(())
}
