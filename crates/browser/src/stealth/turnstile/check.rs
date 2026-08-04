//! Turnstile 绕过状态检测。

use crate::page::Page;
use serde_json::json;
use wisp_core::error::Result;

async fn has_cf_clearance(page: &Page) -> bool {
    let Ok(cookies) = page.cmd("Network.getCookies", json!({})).await else {
        return false;
    };
    cookies
        .pointer("/cookies")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .any(|c| c.get("name").and_then(|n| n.as_str()) == Some("cf_clearance"))
        })
        .unwrap_or(false)
}

async fn main_frame_id(page: &Page) -> Result<String> {
    let frame_tree = page.cmd("Page.getFrameTree", json!({})).await?;
    Ok(frame_tree
        .pointer("/frameTree/frame/id")
        .and_then(|id| id.as_str())
        .unwrap_or("")
        .to_string())
}

async fn evaluate_challenge_title(page: &Page, frame_id: &str) -> Result<Option<bool>> {
    let world = match page
        .cmd(
            "Page.createIsolatedWorld",
            json!({
                "frameId": frame_id,
                "grantUniveralAccess": true,
                "worldName": "cf_check"
            }),
        )
        .await
    {
        Ok(w) => w,
        Err(_) => return Ok(None),
    };
    let Some(ctx_id) = world.get("executionContextId").and_then(|id| id.as_u64()) else {
        return Ok(None);
    };
    let check_js = r#"(() => {
        const title = document.title || '';
        const onChallenge = title.includes('Just a moment') ||
                            title.includes('请稍候') ||
                            title.includes('请稍後') ||
                            title.includes('Attention Required') ||
                            title === '';
        return !onChallenge;
    })()"#;
    let result = match page
        .cmd(
            "Runtime.evaluate",
            json!({
                "expression": check_js,
                "contextId": ctx_id,
                "returnByValue": true,
                "awaitPromise": false
            }),
        )
        .await
    {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    Ok(Some(
        result
            .pointer("/result/value")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    ))
}

pub(super) async fn check_bypassed(page: &Page) -> Result<bool> {
    let has_cf_clearance = has_cf_clearance(page).await;
    let frame_id = main_frame_id(page).await?;
    if frame_id.is_empty() {
        return Ok(has_cf_clearance);
    }
    match evaluate_challenge_title(page, &frame_id).await? {
        Some(title_ok) => Ok(title_ok),
        None => Ok(has_cf_clearance),
    }
}
