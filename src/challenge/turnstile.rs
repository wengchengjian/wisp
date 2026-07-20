//! Cloudflare Turnstile challenge solving via CDP shadow DOM piercing.
//!
//! Key technique: Turnstile renders inside a closed shadow DOM.
//! Normal JS cannot access it. We use CDP DOM.getDocument(pierce=true)
//! to find the iframe node, then DOM.getContentQuads for coordinates.

use std::time::Duration;
use serde_json::{json, Value};

use crate::error::{WispError, Result};
use crate::page::Page;

/// Solve a Cloudflare Turnstile challenge on the given page.
///
/// Strategy (from banzhu-rs, proven effective):
/// 1. Passive wait for JS challenge phase (2s)
/// 2. Every 2s: CDP pierce shadow DOM -> find iframe -> get coords -> click checkbox
/// 3. Check for cf_clearance cookie or page content change
/// 4. Timeout after `timeout` duration
pub async fn solve_turnstile(page: &Page, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    let passive_wait = Duration::from_secs(3);
    let click_interval = Duration::from_secs(2);
    let mut click_count: u32 = 0;
    let mut last_click = tokio::time::Instant::now();
    let start = tokio::time::Instant::now();

    // Enable DOM domain for pierce operations
    let _ = page.cmd("DOM.enable", json!({})).await;

    loop {
        let elapsed = start.elapsed();
        if tokio::time::Instant::now() > deadline {
            return Err(WispError::Timeout(format!(
                "Turnstile not solved after {:.0}s ({} clicks)",
                elapsed.as_secs_f64(),
                click_count
            )));
        }

        // Check if challenge is already passed (cf_clearance cookie or content change)
        if check_bypassed(page).await? {
            tracing::info!("Turnstile passed after {:.0}s, {} clicks", elapsed.as_secs_f64(), click_count);
            // Wait a moment for page to finish redirecting
            tokio::time::sleep(Duration::from_secs(2)).await;
            return Ok(());
        }

        // After passive wait, try clicking every click_interval
        if elapsed > passive_wait && last_click.elapsed() >= click_interval {
            click_count += 1;
            let clicked = try_click_turnstile_cdp(page, click_count).await;
            if clicked {
                tracing::debug!("[click #{}] Turnstile click dispatched", click_count);
            } else if click_count <= 3 || click_count % 5 == 0 {
                tracing::debug!("[click #{}] Turnstile iframe not found", click_count);
            }
            last_click = tokio::time::Instant::now();
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Check if the CF challenge has been bypassed.
async fn check_bypassed(page: &Page) -> Result<bool> {
    // Check cf_clearance cookie via CDP (httpOnly cookies not visible to document.cookie!)
    if let Ok(resp) = page.cmd("Network.getCookies", json!({})).await {
        let has_cf = resp.pointer("/cookies")
            .and_then(|c| c.as_array())
            .map(|arr| arr.iter().any(|c| c.get("name").and_then(|n| n.as_str()) == Some("cf_clearance")))
            .unwrap_or(false);
        if has_cf {
            return Ok(true);
        }
    }

    // Fallback: check if challenge elements are gone and page has real content
    let content_check = page.evaluate(r#"(() => {
        const body = document.body ? document.body.innerHTML : '';
        const hasCf = body.includes('cf-chl-widget') ||
                      body.includes('challenge-platform') ||
                      body.includes('cf-browser-verification');
        const title = document.title || '';
        const onChallenge = title.includes('Just a moment') || title.includes('\u8bf7\u7a0d\u5019');
        return !hasCf && !onChallenge && body.length > 1000;
    })()"#).await?;

    Ok(content_check.as_bool().unwrap_or(false))
}

/// Use CDP to pierce shadow DOM, find Turnstile iframe, and click it.
///
/// This is the core technique: JS cannot access closed shadow roots,
/// but CDP DOM.getDocument(pierce=true) can traverse them.
async fn try_click_turnstile_cdp(page: &Page, round: u32) -> bool {
    // Step 1: Get full DOM tree with shadow DOM piercing
    let doc = match page.cmd("DOM.getDocument", json!({
        "depth": 200,
        "pierce": true
    })).await {
        Ok(r) => r,
        Err(_) => return false,
    };

    // Step 2: Recursively find Turnstile iframe nodeId
    let root = match doc.get("root") {
        Some(r) => r,
        None => return false,
    };

    let iframe_node_id = match find_turnstile_node(root) {
        Some(id) => id,
        None => return false,
    };

    // Step 3: Get iframe viewport coordinates via GetContentQuads
    let quads_result = match page.cmd("DOM.getContentQuads", json!({
        "nodeId": iframe_node_id
    })).await {
        Ok(r) => r,
        Err(_) => return false,
    };

    let quads = match quads_result.get("quads").and_then(|q| q.as_array()) {
        Some(q) if !q.is_empty() => q,
        _ => return false,
    };

    let quad = match quads[0].as_array() {
        Some(q) if q.len() >= 8 => q,
        _ => return false,
    };

    let iframe_x = quad[0].as_f64().unwrap_or(0.0);
    let iframe_y = quad[1].as_f64().unwrap_or(0.0);
    let iframe_h = quad[5].as_f64().unwrap_or(65.0) - iframe_y;

    // Turnstile checkbox is at left ~32px, vertically centered
    // Add small per-round jitter to avoid detection
    let cx = iframe_x + 32.0 + ((round as f64 % 5.0) - 2.0) * 3.0;
    let cy = iframe_y + iframe_h / 2.0 + ((round as f64 % 3.0) - 1.0) * 2.0;

    if round <= 3 {
        tracing::debug!(
            "[click #{}] iframe nodeId={}, pos=({:.0},{:.0}), clicking ({:.0},{:.0})",
            round, iframe_node_id, iframe_x, iframe_y, cx, cy
        );
    }

    // Step 4: Simulate mouse movement (ease-out deceleration)
    let steps = 10;
    let sx = cx - 50.0 + ((round as f64 % 7.0) - 3.0) * 15.0;
    let sy = cy - 40.0 + ((round as f64 % 5.0) - 2.0) * 12.0;

    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let ease = 1.0 - (1.0 - t) * (1.0 - t); // ease-out
        let mx = sx + (cx - sx) * ease;
        let my = sy + (cy - sy) * ease;

        let _ = page.cmd("Input.dispatchMouseEvent", json!({
            "type": "mouseMoved",
            "x": mx,
            "y": my,
            "modifiers": 0,
            "buttons": 0
        })).await;

        tokio::time::sleep(Duration::from_millis(10 + (i as u64 * 5).min(40))).await;
    }

    // Step 5: Click (press + release)
    let _ = page.cmd("Input.dispatchMouseEvent", json!({
        "type": "mousePressed",
        "x": cx,
        "y": cy,
        "button": "left",
        "clickCount": 1,
        "modifiers": 0,
        "buttons": 0
    })).await;

    tokio::time::sleep(Duration::from_millis(60 + (round as u64 * 13) % 50)).await;

    let _ = page.cmd("Input.dispatchMouseEvent", json!({
        "type": "mouseReleased",
        "x": cx,
        "y": cy,
        "button": "left",
        "clickCount": 1,
        "modifiers": 0,
        "buttons": 0
    })).await;

    true
}

/// Recursively search DOM tree (including shadow roots) for Turnstile iframe.
/// Returns the nodeId if found.
fn find_turnstile_node(node: &Value) -> Option<u32> {
    let node_name = node.get("nodeName").and_then(|n| n.as_str()).unwrap_or("");
    let attributes = node.get("attributes").and_then(|a| a.as_array());

    // Check if this is a Turnstile iframe
    if node_name.eq_ignore_ascii_case("IFRAME") {
        if let Some(attrs) = attributes {
            let is_turnstile = attrs.chunks(2).any(|pair| {
                if pair.len() == 2 {
                    let key = pair[0].as_str().unwrap_or("");
                    let val = pair[1].as_str().unwrap_or("");
                    (key == "src" && val.contains("challenges.cloudflare.com"))
                        || (key == "id" && val.contains("cf-chl-widget"))
                } else {
                    false
                }
            });
            if is_turnstile {
                return node.get("nodeId").and_then(|id| id.as_u64()).map(|id| id as u32);
            }
        }
    }

    // Recurse into children
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for child in children {
            if let Some(id) = find_turnstile_node(child) {
                return Some(id);
            }
        }
    }

    // Recurse into shadow roots (pierces closed shadow DOM!)
    if let Some(shadow_roots) = node.get("shadowRoots").and_then(|s| s.as_array()) {
        for sr in shadow_roots {
            if let Some(sr_children) = sr.get("children").and_then(|c| c.as_array()) {
                for sr_child in sr_children {
                    if let Some(id) = find_turnstile_node(sr_child) {
                        return Some(id);
                    }
                }
            }
        }
    }

    // Recurse into iframe contentDocument
    if let Some(content_doc) = node.get("contentDocument") {
        if let Some(id) = find_turnstile_node(content_doc) {
            return Some(id);
        }
    }

    None
}
