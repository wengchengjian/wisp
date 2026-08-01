//! Turnstile iframe 定位与点击。

use serde_json::{json, Value};
use std::time::Duration;

use super::config::TurnstileConfig;
use wisp_browser::page::Page;

async fn turnstile_iframe_node(page: &Page, cfg: &TurnstileConfig) -> Option<u32> {
    let doc = page
        .cmd(
            "DOM.getDocument",
            json!({ "depth": cfg.dom_depth, "pierce": true }),
        )
        .await
        .ok()?;
    find_turnstile_node(doc.get("root")?)
}

async fn iframe_click_geometry(page: &Page, node_id: u32) -> Option<(f64, f64, f64)> {
    let quads_result = page
        .cmd("DOM.getContentQuads", json!({ "nodeId": node_id }))
        .await
        .ok()?;
    let quads = quads_result.get("quads").and_then(|q| q.as_array())?;
    let quad = quads.first()?.as_array()?;
    if quad.len() < 8 {
        return None;
    }
    let x = quad[0].as_f64()?;
    let y = quad[1].as_f64()?;
    let h = quad[5].as_f64().unwrap_or(65.0) - y;
    Some((x, y, h))
}

fn turnstile_click_position(round: u32, x: f64, y: f64, h: f64) -> (f64, f64) {
    let positions: Vec<(f64, f64)> = vec![
        (x + 32.0, y + h / 2.0),
        (x + 28.0, y + h / 2.0),
        (x + 36.0, y + h / 2.0),
        (x + 32.0, y + h * 0.4),
        (x + 32.0, y + h * 0.6),
    ];
    let idx = (round as usize) % positions.len();
    positions[idx]
}

async fn move_mouse_to(page: &Page, cfg: &TurnstileConfig, round: u32, cx: f64, cy: f64) {
    let steps = cfg.mouse_steps;
    let sx = cx - 50.0 + ((round as f64 % 7.0) - 3.0) * 15.0;
    let sy = cy - 40.0 + ((round as f64 % 5.0) - 2.0) * 12.0;
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let ease = 1.0 - (1.0 - t) * (1.0 - t);
        let mx = sx + (cx - sx) * ease;
        let my = sy + (cy - sy) * ease;
        let _ = page
            .cmd(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseMoved", "x": mx, "y": my, "modifiers": 0, "buttons": 0 }),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(cfg.mouse_step_delay_ms)).await;
    }
}

async fn press_and_release_turnstile(page: &Page, cfg: &TurnstileConfig, cx: f64, cy: f64) {
    let _ = page
        .cmd(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mousePressed",
                "x": cx,
                "y": cy,
                "button": "left",
                "clickCount": 1,
                "modifiers": 0,
                "buttons": 1
            }),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(cfg.click_hold_ms)).await;
    let _ = page
        .cmd(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseReleased",
                "x": cx,
                "y": cy,
                "button": "left",
                "clickCount": 1,
                "modifiers": 0,
                "buttons": 0
            }),
        )
        .await;
}

/// Use CDP to pierce shadow DOM, find Turnstile iframe, and click it.
pub(super) async fn try_click_turnstile_cdp(
    page: &Page,
    round: u32,
    cfg: &TurnstileConfig,
) -> bool {
    let Some(iframe_node_id) = turnstile_iframe_node(page, cfg).await else {
        return false;
    };
    let Some((x, y, h)) = iframe_click_geometry(page, iframe_node_id).await else {
        return false;
    };
    let (cx, cy) = turnstile_click_position(round, x, y, h);
    if round <= 3 {
        tracing::debug!(
            "[click #{}] iframe nodeId={}, pos=({:.0},{:.0}), clicking ({:.0},{:.0})",
            round,
            iframe_node_id,
            x,
            y,
            cx,
            cy
        );
    }
    move_mouse_to(page, cfg, round, cx, cy).await;
    press_and_release_turnstile(page, cfg, cx, cy).await;
    true
}

fn is_turnstile_iframe(node: &Value) -> bool {
    if !node
        .get("nodeName")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .eq_ignore_ascii_case("IFRAME")
    {
        return false;
    }
    let Some(attrs) = node.get("attributes").and_then(|a| a.as_array()) else {
        return false;
    };
    attrs.chunks(2).any(|pair| {
        if pair.len() != 2 {
            return false;
        }
        let key = pair[0].as_str().unwrap_or("");
        let val = pair[1].as_str().unwrap_or("");
        (key == "src" && val.contains("challenges.cloudflare.com"))
            || (key == "id" && val.contains("cf-chl-widget"))
    })
}

/// Recursively search DOM tree (including shadow roots) for Turnstile iframe.
/// Returns the nodeId if found.
fn find_turnstile_node(node: &Value) -> Option<u32> {
    if is_turnstile_iframe(node) {
        return node
            .get("nodeId")
            .and_then(|id| id.as_u64())
            .map(|id| id as u32);
    }
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for child in children {
            if let Some(id) = find_turnstile_node(child) {
                return Some(id);
            }
        }
    }
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
    if let Some(content_doc) = node.get("contentDocument") {
        if let Some(id) = find_turnstile_node(content_doc) {
            return Some(id);
        }
    }
    None
}
