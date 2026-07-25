//! Cloudflare Turnstile challenge solving via CDP shadow DOM piercing.
//!
//! Key technique: Turnstile renders inside a closed shadow DOM.
//! Normal JS cannot access it. We use CDP DOM.getDocument(pierce=true)
//! to find the iframe node, then DOM.getContentQuads for coordinates.

use std::time::Duration;
use serde_json::{json, Value};

use crate::error::{WispError, Result};
use crate::browser::page::Page;

/// Turnstile 解决器可调参数。
///
/// 用户可根据网络环境和 CF 策略调整这些参数以优化速度/成功率。
#[derive(Debug, Clone)]
pub struct TurnstileConfig {
    /// 首次点击前的被动等待（等 Turnstile widget 加载）。
    /// 默认 500ms。网络慢可调高，快可调低。
    pub passive_wait_ms: u64,
    /// 点击间隔（第一次点击失败后重试间隔）。
    /// 默认 2000ms。
    pub click_interval_ms: u64,
    /// 循环检测间隔（检查挑战是否通过）。
    /// 默认 200ms。调低可更快检测到 bypass。
    pub poll_interval_ms: u64,
    /// 鼠标移动步数（模拟人类轨迹）。
    /// 默认 5。调低更快但可能被检测。
    pub mouse_steps: u32,
    /// 每步鼠标移动延迟 (ms)。
    /// 默认 15ms。
    pub mouse_step_delay_ms: u64,
    /// 鼠标按下到释放的延迟 (ms)。
    /// 默认 60ms。
    pub click_hold_ms: u64,
    /// DOM 查询深度（pierce shadow DOM）。
    /// 默认 10。调低更快但可能找不到 iframe。
    pub dom_depth: u32,
}

impl Default for TurnstileConfig {
    fn default() -> Self {
        Self {
            passive_wait_ms: 200,
            click_interval_ms: 1500,
            poll_interval_ms: 100,
            mouse_steps: 3,
            mouse_step_delay_ms: 5,
            click_hold_ms: 30,
            dom_depth: 10,
        }
    }
}

/// Solve a Cloudflare Turnstile challenge on the given page.
pub async fn solve_turnstile(page: &Page, timeout: Duration) -> Result<()> {
    solve_turnstile_with_config(page, timeout, &TurnstileConfig::default()).await
}

/// 使用自定义配置解决 Turnstile 挑战。
pub async fn solve_turnstile_with_config(page: &Page, timeout: Duration, cfg: &TurnstileConfig) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    let passive_wait = Duration::from_millis(cfg.passive_wait_ms);
    let click_interval = Duration::from_millis(cfg.click_interval_ms);
    let mut click_count: u32 = 0;
    let mut last_click = tokio::time::Instant::now();
    let start = tokio::time::Instant::now();

    loop {
        let elapsed = start.elapsed();
        if tokio::time::Instant::now() > deadline {
            return Err(WispError::Timeout(format!(
                "Turnstile not solved after {:.0}s ({} clicks)",
                elapsed.as_secs_f64(),
                click_count
            )));
        }

        // Check if challenge is already passed
        let t0 = tokio::time::Instant::now();
        match check_bypassed(page).await {
            Ok(true) => {
                println!("[turnstile] {:.1}s: bypassed detected (check took {:.0}ms), {} clicks",
                    elapsed.as_secs_f64(), t0.elapsed().as_millis(), click_count);
                return Ok(());
            }
            Ok(false) => {}
            Err(e) => {
                println!("[turnstile] {:.1}s: check error: {}", elapsed.as_secs_f64(), e);
                tokio::time::sleep(Duration::from_millis(500)).await;
                if check_bypassed(page).await.unwrap_or(false) {
                    println!("[turnstile] {:.1}s: bypassed after retry", start.elapsed().as_secs_f64());
                    return Ok(());
                }
            }
        }

        // After passive wait, try clicking every click_interval
        if elapsed > passive_wait && last_click.elapsed() >= click_interval {
            click_count += 1;
            let t1 = tokio::time::Instant::now();
            let clicked = try_click_turnstile_cdp(page, click_count, cfg).await;
            println!("[turnstile] {:.1}s: click #{} {} ({:.0}ms)",
                elapsed.as_secs_f64(), click_count,
                if clicked { "OK" } else { "iframe not found" },
                t1.elapsed().as_millis());
            last_click = tokio::time::Instant::now();
        }

        tokio::time::sleep(Duration::from_millis(cfg.poll_interval_ms)).await;
    }
}

/// Check if the CF challenge has been bypassed.
///
/// 快速检测策略：
/// 1. cf_clearance cookie 存在 + 标题非挑战页 = 立即返回 true
/// 2. 页面内容检查作为备用
async fn check_bypassed(page: &Page) -> Result<bool> {
    // 检查 cf_clearance cookie
    let cookies_result = page.cmd("Network.getCookies", json!({})).await;
    let has_cf_clearance = if let Ok(cookies) = cookies_result {
        cookies.pointer("/cookies")
            .and_then(|c| c.as_array())
            .map(|arr| arr.iter().any(|c| {
                c.get("name").and_then(|n| n.as_str()) == Some("cf_clearance")
            }))
            .unwrap_or(false)
    } else {
        false
    };

    // 获取 frame tree
    let frame_tree = page.cmd("Page.getFrameTree", json!({})).await?;
    let frame_id = frame_tree.pointer("/frameTree/frame/id")
        .and_then(|id| id.as_str())
        .unwrap_or("")
        .to_string();

    if frame_id.is_empty() {
        return Ok(has_cf_clearance);
    }

    // 创建 isolated world 检查标题
    let world = page.cmd("Page.createIsolatedWorld", json!({
        "frameId": frame_id,
        "grantUniveralAccess": true,
        "worldName": "cf_check"
    })).await;

    let context_id = match world {
        Ok(w) => w.get("executionContextId").and_then(|id| id.as_u64()),
        Err(_) => None,
    };

    match context_id {
        Some(ctx_id) => {
            // 只检查标题（快速，不依赖 body 加载完成）
            let check_js = r#"(() => {
                const title = document.title || '';
                const onChallenge = title.includes('Just a moment') ||
                                    title.includes('请稍候') ||
                                    title.includes('请稍後') ||
                                    title.includes('Attention Required') ||
                                    title === '';
                return !onChallenge;
            })()"#;

            let result = page.cmd("Runtime.evaluate", json!({
                "expression": check_js,
                "contextId": ctx_id,
                "returnByValue": true,
                "awaitPromise": false
            })).await;

            match result {
                Ok(r) => {
                    let title_ok = r.pointer("/result/value").and_then(|v| v.as_bool()).unwrap_or(false);
                    // cf_clearance + 标题非挑战页 = 确认绕过
                    // 无 cf_clearance 但标题非挑战页 = 也绕过
                    Ok(title_ok)
                }
                Err(_) => Ok(has_cf_clearance),
            }
        }
        None => Ok(has_cf_clearance),
    }
}

/// Use CDP to pierce shadow DOM, find Turnstile iframe, and click it.
async fn try_click_turnstile_cdp(page: &Page, round: u32, cfg: &TurnstileConfig) -> bool {
    // Step 1: Get full DOM tree with shadow DOM piercing
    let doc = match page.cmd("DOM.getDocument", json!({
        "depth": cfg.dom_depth,
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
    // Try multiple positions to account for different widget sizes
    let positions: Vec<(f64, f64)> = vec![
        (iframe_x + 32.0, iframe_y + iframe_h / 2.0),   // standard checkbox position
        (iframe_x + 28.0, iframe_y + iframe_h / 2.0),   // slightly left
        (iframe_x + 36.0, iframe_y + iframe_h / 2.0),   // slightly right
        (iframe_x + 32.0, iframe_y + iframe_h * 0.4),   // slightly up
        (iframe_x + 32.0, iframe_y + iframe_h * 0.6),   // slightly down
    ];
    let pos_idx = (round as usize) % positions.len();
    let (cx, cy) = positions[pos_idx];

    if round <= 3 {
        tracing::debug!(
            "[click #{}] iframe nodeId={}, pos=({:.0},{:.0}), clicking ({:.0},{:.0})",
            round, iframe_node_id, iframe_x, iframe_y, cx, cy
        );
    }

    // Step 4: Simulate mouse movement (ease-out deceleration)
    let steps = cfg.mouse_steps;
    let sx = cx - 50.0 + ((round as f64 % 7.0) - 3.0) * 15.0;
    let sy = cy - 40.0 + ((round as f64 % 5.0) - 2.0) * 12.0;

    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let ease = 1.0 - (1.0 - t) * (1.0 - t);
        let mx = sx + (cx - sx) * ease;
        let my = sy + (cy - sy) * ease;

        let _ = page.cmd("Input.dispatchMouseEvent", json!({
            "type": "mouseMoved",
            "x": mx,
            "y": my,
            "modifiers": 0,
            "buttons": 0
        })).await;

        tokio::time::sleep(Duration::from_millis(cfg.mouse_step_delay_ms)).await;
    }

    // Step 5: Click (press + release)
    let _ = page.cmd("Input.dispatchMouseEvent", json!({
        "type": "mousePressed",
        "x": cx,
        "y": cy,
        "button": "left",
        "clickCount": 1,
        "modifiers": 0,
        "buttons": 1
    })).await;

    tokio::time::sleep(Duration::from_millis(cfg.click_hold_ms)).await;

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
