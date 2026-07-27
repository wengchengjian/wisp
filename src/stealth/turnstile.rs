//! Cloudflare Turnstile challenge solving via CDP shadow DOM piercing.
//!
//! Key technique: Turnstile renders inside a closed shadow DOM.
//! Normal JS cannot access it. We use CDP DOM.getDocument(pierce=true)
//! to find the iframe node, then DOM.getContentQuads for coordinates.

use serde_json::{json, Value};
use std::time::Duration;

use crate::browser::page::Page;
use crate::error::{Result, WispError};

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
pub async fn solve_turnstile_with_config(
    page: &Page,
    timeout: Duration,
    cfg: &TurnstileConfig,
) -> Result<()> {
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
                println!(
                    "[turnstile] {:.1}s: bypassed detected (check took {:.0}ms), {} clicks",
                    elapsed.as_secs_f64(),
                    t0.elapsed().as_millis(),
                    click_count
                );
                return Ok(());
            }
            Ok(false) => {}
            Err(e) => {
                println!(
                    "[turnstile] {:.1}s: check error: {}",
                    elapsed.as_secs_f64(),
                    e
                );
                tokio::time::sleep(Duration::from_millis(500)).await;
                if check_bypassed(page).await.unwrap_or(false) {
                    println!(
                        "[turnstile] {:.1}s: bypassed after retry",
                        start.elapsed().as_secs_f64()
                    );
                    return Ok(());
                }
            }
        }

        // After passive wait, try clicking every click_interval
        if elapsed > passive_wait && last_click.elapsed() >= click_interval {
            click_count += 1;
            let t1 = tokio::time::Instant::now();
            let clicked = try_click_turnstile_cdp(page, click_count, cfg).await;
            println!(
                "[turnstile] {:.1}s: click #{} {} ({:.0}ms)",
                elapsed.as_secs_f64(),
                click_count,
                if clicked { "OK" } else { "iframe not found" },
                t1.elapsed().as_millis()
            );
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
    let has_cf_clearance = has_cf_clearance_cookie(page).await;

    let Some(frame_id) = get_main_frame_id(page).await? else {
        return Ok(has_cf_clearance);
    };

    check_title_not_challenge(page, &frame_id, has_cf_clearance).await
}

/// 检查 cf_clearance cookie 是否存在。
async fn has_cf_clearance_cookie(page: &Page) -> bool {
    let Ok(cookies) = page.cmd("Network.getCookies", json!({})).await else {
        return false;
    };
    cookies
        .pointer("/cookies")
        .and_then(|c| c.as_array())
        .is_some_and(|arr| {
            arr.iter()
                .any(|c| c.get("name").and_then(|n| n.as_str()) == Some("cf_clearance"))
        })
}

/// 获取主 frame id，空则返回 None。
async fn get_main_frame_id(page: &Page) -> Result<Option<String>> {
    let frame_tree = page.cmd("Page.getFrameTree", json!({})).await?;
    let frame_id = frame_tree
        .pointer("/frameTree/frame/id")
        .and_then(|id| id.as_str())
        .unwrap_or("");
    if frame_id.is_empty() {
        Ok(None)
    } else {
        Ok(Some(frame_id.to_string()))
    }
}

/// 在 isolated world 中检查标题是否为非挑战页。
/// cf_clearance + 标题非挑战页 = 确认绕过；无 cf_clearance 但标题非挑战页 = 也绕过。
async fn check_title_not_challenge(
    page: &Page,
    frame_id: &str,
    has_cf_clearance: bool,
) -> Result<bool> {
    let world = page
        .cmd(
            "Page.createIsolatedWorld",
            json!({
                "frameId": frame_id,
                "grantUniveralAccess": true,
                "worldName": "cf_check"
            }),
        )
        .await;

    let context_id = match world {
        Ok(w) => w
            .get("executionContextId")
            .and_then(serde_json::Value::as_u64),
        Err(_) => None,
    };

    let Some(ctx_id) = context_id else {
        return Ok(has_cf_clearance);
    };

    // 只检查标题（快速，不依赖 body 加载完成）
    let check_js = r"(() => {
        const title = document.title || '';
        const onChallenge = title.includes('Just a moment') ||
                            title.includes('请稍候') ||
                            title.includes('请稍後') ||
                            title.includes('Attention Required') ||
                            title === '';
        return !onChallenge;
    })()";

    let result = page
        .cmd(
            "Runtime.evaluate",
            json!({
                "expression": check_js,
                "contextId": ctx_id,
                "returnByValue": true,
                "awaitPromise": false
            }),
        )
        .await;

    match result {
        Ok(r) => Ok(r
            .pointer("/result/value")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)),
        Err(_) => Ok(has_cf_clearance),
    }
}

/// Use CDP to pierce shadow DOM, find Turnstile iframe, and click it.
async fn try_click_turnstile_cdp(page: &Page, round: u32, cfg: &TurnstileConfig) -> bool {
    // Step 1+2: 获取 DOM 树（穿透 shadow DOM）并找到 Turnstile iframe
    let Some(iframe_node_id) = find_turnstile_iframe(page, cfg).await else {
        return false;
    };

    // Step 3: 获取 iframe 坐标并计算点击位置（多轮轮换）
    let Some((cx, cy)) = get_click_position(page, iframe_node_id, round).await else {
        return false;
    };

    // Step 4+5: 模拟鼠标移动 + 点击
    simulate_move_and_click(page, cx, cy, round, cfg).await;

    true
}

/// 获取 DOM 树（穿透 shadow DOM）并递归查找 Turnstile iframe nodeId。
async fn find_turnstile_iframe(page: &Page, cfg: &TurnstileConfig) -> Option<u32> {
    let doc = page
        .cmd(
            "DOM.getDocument",
            json!({
                "depth": cfg.dom_depth,
                "pierce": true
            }),
        )
        .await
        .ok()?;
    let root = doc.get("root")?;
    find_turnstile_node(root)
}

/// 获取 iframe 坐标，计算点击位置（5 个候选位置按 round 轮换）。
async fn get_click_position(page: &Page, iframe_node_id: u32, round: u32) -> Option<(f64, f64)> {
    let quads_result = page
        .cmd(
            "DOM.getContentQuads",
            json!({
                "nodeId": iframe_node_id
            }),
        )
        .await
        .ok()?;

    let quads = quads_result.get("quads").and_then(|q| q.as_array())?;
    let quad = quads.first()?.as_array()?;
    if quad.len() < 8 {
        return None;
    }

    let iframe_x = quad[0].as_f64().unwrap_or(0.0);
    let iframe_y = quad[1].as_f64().unwrap_or(0.0);
    let iframe_h = quad[5].as_f64().unwrap_or(65.0) - iframe_y;

    // Turnstile checkbox 在左侧 ~32px，垂直居中；多个位置应对不同 widget 尺寸
    let positions: [(f64, f64); 5] = [
        (iframe_x + 32.0, iframe_y + iframe_h / 2.0), // standard checkbox position
        (iframe_x + 28.0, iframe_y + iframe_h / 2.0), // slightly left
        (iframe_x + 36.0, iframe_y + iframe_h / 2.0), // slightly right
        (iframe_x + 32.0, iframe_y + iframe_h * 0.4),  // slightly up
        (iframe_x + 32.0, iframe_y + iframe_h * 0.6),  // slightly down
    ];
    let pos_idx = (round as usize) % positions.len();

    if round <= 3 {
        tracing::debug!(
            "[click #{}] iframe nodeId={}, pos=({:.0},{:.0}), clicking ({:.0},{:.0})",
            round,
            iframe_node_id,
            iframe_x,
            iframe_y,
            positions[pos_idx].0,
            positions[pos_idx].1
        );
    }

    Some(positions[pos_idx])
}

/// 模拟鼠标移动（ease-out 减速）到目标位置并点击（press + release）。
async fn simulate_move_and_click(
    page: &Page,
    cx: f64,
    cy: f64,
    round: u32,
    cfg: &TurnstileConfig,
) {
    // Step 4: 模拟鼠标移动（ease-out deceleration）
    let steps = cfg.mouse_steps;
    let sx = cx - 50.0 + ((f64::from(round) % 7.0) - 3.0) * 15.0;
    let sy = cy - 40.0 + ((f64::from(round) % 5.0) - 2.0) * 12.0;

    for i in 0..=steps {
        let t = f64::from(i) / f64::from(steps);
        let ease = 1.0 - (1.0 - t) * (1.0 - t);
        let mx = sx + (cx - sx) * ease;
        let my = sy + (cy - sy) * ease;

        let _ = page
            .cmd(
                "Input.dispatchMouseEvent",
                json!({
                    "type": "mouseMoved",
                    "x": mx,
                    "y": my,
                    "modifiers": 0,
                    "buttons": 0
                }),
            )
            .await;

        tokio::time::sleep(Duration::from_millis(cfg.mouse_step_delay_ms)).await;
    }

    // Step 5: 点击（press + release）
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
                return node
                    .get("nodeId")
                    .and_then(serde_json::Value::as_u64)
                    .map(|id| id as u32);
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
