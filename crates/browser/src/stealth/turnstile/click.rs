//! Turnstile iframe 定位与点击。

use serde_json::{Value, json};
use std::time::Duration;

use crate::page::Page;
use wisp_core::stealth::TurnstileConfig;

use rand::rngs::{SmallRng, SysRng};
use rand::{RngExt, SeedableRng};

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

/// 拟人化鼠标移动：从随机起点沿贝塞尔曲线移动到目标，步数与每步延迟随机。
/// 相比固定直线移动，轨迹更接近真实用户，降低被 CF 行为检测的概率。
async fn move_mouse_human(page: &Page, cx: f64, cy: f64) {
    let mut rng = SmallRng::try_from_rng(&mut SysRng).expect("OS RNG failed");
    let start_x = rng.random_range(cx - 90.0..cx - 30.0);
    let start_y = rng.random_range(cy - 60.0..cy - 10.0);
    let cp1_x = start_x + (cx - start_x) * 0.3 + rng.random_range(-40.0..40.0);
    let cp1_y = start_y + (cy - start_y) * 0.3 + rng.random_range(-40.0..40.0);
    let cp2_x = start_x + (cx - start_x) * 0.7 + rng.random_range(-25.0..25.0);
    let cp2_y = start_y + (cy - start_y) * 0.7 + rng.random_range(-25.0..25.0);
    let steps = rng.random_range(10..=22);
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let x = cubic_bezier(start_x, cp1_x, cp2_x, cx, t);
        let y = cubic_bezier(start_y, cp1_y, cp2_y, cy, t);
        let _ = page
            .cmd(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseMoved", "x": x, "y": y, "modifiers": 0, "buttons": 0 }),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(rng.random_range(4..=18))).await;
    }
}

/// 快速直线移动（非 human_mode）：固定步数与延迟，追求速度。
async fn move_mouse_fast(page: &Page, cfg: &TurnstileConfig, round: u32, cx: f64, cy: f64) {
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
    // 拟人化：点击前随机停留，按下位置加随机小偏移，避免每次都精确命中中心。
    let mut rng = SmallRng::try_from_rng(&mut SysRng).expect("OS RNG failed");
    if cfg.human_mode {
        tokio::time::sleep(Duration::from_millis(rng.random_range(40..=160))).await;
    }
    let (bx, by) = if cfg.human_mode {
        (
            cx + rng.random_range(-2.0..2.0),
            cy + rng.random_range(-2.0..2.0),
        )
    } else {
        (cx, cy)
    };
    let _ = page
        .cmd(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mousePressed",
                "x": bx,
                "y": by,
                "button": "left",
                "clickCount": 1,
                "modifiers": 0,
                "buttons": 1
            }),
        )
        .await;
    let hold = if cfg.human_mode {
        rng.random_range(45..=110)
    } else {
        cfg.click_hold_ms
    };
    tokio::time::sleep(Duration::from_millis(hold)).await;
    let _ = page
        .cmd(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseReleased",
                "x": bx,
                "y": by,
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
    if cfg.human_mode {
        move_mouse_human(page, cx, cy).await;
    } else {
        move_mouse_fast(page, cfg, round, cx, cy).await;
    }
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
    if let Some(content_doc) = node.get("contentDocument")
        && let Some(id) = find_turnstile_node(content_doc)
    {
        return Some(id);
    }
    None
}

/// Cubic bezier interpolation used by the human-like mouse path.
fn cubic_bezier(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
}
