//! Cloudflare Turnstile challenge solving.
//!
//! Strategy:
//! 1. Wait for the Turnstile iframe to load
//! 2. Check if it auto-solves (invisible/managed mode)
//! 3. If interactive: locate the checkbox and simulate a human click
//! 4. Wait for cf-clearance cookie or page navigation

use std::time::Duration;
use serde_json::json;

use crate::error::{WispError, Result};
use crate::page::Page;
use crate::human::HumanBehavior;

/// Solve a Cloudflare Turnstile challenge on the given page.
pub async fn solve_turnstile(page: &Page, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    let human = HumanBehavior::new(page);

    // Wait for the Turnstile iframe to appear
    wait_for_turnstile_iframe(page, Duration::from_secs(10)).await?;

    // Give it a moment to initialize
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Check if it auto-solved (invisible mode or managed mode with good score)
    if is_turnstile_solved(page).await? {
        return Ok(());
    }

    // Try clicking the Turnstile checkbox
    // The checkbox is inside an iframe from challenges.cloudflare.com
    let clicked = try_click_turnstile(page, &human).await?;

    if clicked {
        // Wait for the challenge to resolve after clicking
        wait_for_resolution(page, deadline).await
    } else {
        // Could not find/click the checkbox, wait for auto-resolve
        wait_for_resolution(page, deadline).await
    }
}

/// Wait for the Turnstile iframe to appear in the DOM.
/// Searches through shadow roots since Turnstile renders inside a shadow DOM.
async fn wait_for_turnstile_iframe(page: &Page, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(WispError::Timeout("Turnstile iframe did not appear".into()));
        }

        // Search through shadow roots (Turnstile hides inside shadow DOM)
        let found = page.evaluate(r#"(() => {
            // Direct check
            if (document.querySelector('iframe[src*="challenges.cloudflare.com"]')) return true;
            // Search inside shadow roots
            const allElements = document.querySelectorAll('*');
            for (const el of allElements) {
                if (el.shadowRoot) {
                    const iframe = el.shadowRoot.querySelector('iframe[src*="challenges.cloudflare.com"]') ||
                                   el.shadowRoot.querySelector('iframe[id*="cf-chl"]');
                    if (iframe) return true;
                }
            }
            return false;
        })()"#).await?;

        if found.as_bool().unwrap_or(false) {
            return Ok(());
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Check if the Turnstile has been solved (response token generated).
async fn is_turnstile_solved(page: &Page) -> Result<bool> {
    let result = page.evaluate(r#"(() => {
        // Check for turnstile response input
        const input = document.querySelector('[name="cf-turnstile-response"]');
        if (input && input.value && input.value.length > 0) return true;

        // Check if the widget shows success
        const widget = document.querySelector('.cf-turnstile');
        if (widget && widget.dataset && widget.dataset.status === 'solved') return true;

        // Check for cf-clearance cookie
        return document.cookie.includes('cf_clearance');
    })()"#).await?;

    Ok(result.as_bool().unwrap_or(false))
}

/// Try to click the Turnstile checkbox via CDP.
/// Returns true if a click was performed.
async fn try_click_turnstile(page: &Page, _human: &HumanBehavior<'_>) -> Result<bool> {
    // Get the Turnstile iframe bounding box - search through shadow roots
    let iframe_info = page.evaluate(r#"(() => {
        // Direct search
        let iframe = document.querySelector('iframe[src*="challenges.cloudflare.com"]') ||
                     document.querySelector('iframe[id*="cf-chl"]');
        // Search inside shadow roots
        if (!iframe) {
            const allElements = document.querySelectorAll('*');
            for (const el of allElements) {
                if (el.shadowRoot) {
                    iframe = el.shadowRoot.querySelector('iframe[src*="challenges.cloudflare.com"]') ||
                             el.shadowRoot.querySelector('iframe[id*="cf-chl"]');
                    if (iframe) break;
                }
            }
        }
        if (!iframe) return null;
        const rect = iframe.getBoundingClientRect();
        return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
    })()"#).await?;

    if iframe_info.is_null() {
        return Ok(false);
    }

    let x = iframe_info.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y = iframe_info.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let width = iframe_info.get("width").and_then(|v| v.as_f64()).unwrap_or(300.0);
    let height = iframe_info.get("height").and_then(|v| v.as_f64()).unwrap_or(65.0);

    // The checkbox is typically in the left portion of the iframe
    // Click at approximately (x + 30, y + height/2)
    let click_x = x + 30.0 + rand::random::<f64>() * 5.0;
    let click_y = y + height / 2.0 + rand::random::<f64>() * 3.0;

    // Simulate human-like mouse movement to the checkbox
    let steps = 15;
    let start_x = x + width / 2.0; // Start from center of iframe
    let start_y = y - 50.0; // Start from above

    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let mx = start_x + (click_x - start_x) * t + (rand::random::<f64>() - 0.5) * 3.0;
        let my = start_y + (click_y - start_y) * t + (rand::random::<f64>() - 0.5) * 3.0;

        page.cmd("Input.dispatchMouseEvent", json!({
            "type": "mouseMoved",
            "x": mx,
            "y": my,
        })).await?;

        tokio::time::sleep(Duration::from_millis(10 + rand::random::<u64>() % 15)).await;
    }

    // Pause before clicking
    tokio::time::sleep(Duration::from_millis(100 + rand::random::<u64>() % 200)).await;

    // Click
    page.cmd("Input.dispatchMouseEvent", json!({
        "type": "mousePressed",
        "x": click_x,
        "y": click_y,
        "button": "left",
        "clickCount": 1,
    })).await?;

    tokio::time::sleep(Duration::from_millis(50 + rand::random::<u64>() % 50)).await;

    page.cmd("Input.dispatchMouseEvent", json!({
        "type": "mouseReleased",
        "x": click_x,
        "y": click_y,
        "button": "left",
        "clickCount": 1,
    })).await?;

    Ok(true)
}

/// Wait for the challenge to fully resolve (page navigates or cookie appears).
async fn wait_for_resolution(page: &Page, deadline: tokio::time::Instant) -> Result<()> {
    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(WispError::Timeout("Turnstile challenge did not resolve in time".into()));
        }

        // Check if solved
        if is_turnstile_solved(page).await? {
            // Give the page a moment to redirect
            tokio::time::sleep(Duration::from_millis(2000)).await;
            return Ok(());
        }

        // Check if the challenge elements are gone (page may have redirected)
        let still_present = page.evaluate(r#"(() => {
            return !!document.querySelector('iframe[src*="challenges.cloudflare.com"]') ||
                   !!document.querySelector('.cf-turnstile') ||
                   document.title.includes('Just a moment');
        })()"#).await?;

        if !still_present.as_bool().unwrap_or(true) {
            // Challenge page is gone, we passed
            return Ok(());
        }

        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}
