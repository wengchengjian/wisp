use std::sync::Arc;
use crate::cdp::session::CdpSession;
use crate::error::{PatchrightError, Result};
use crate::page::evaluate::evaluate;

pub async fn click(session: &Arc<CdpSession>, frame_id: &str, selector: &str) -> Result<()> {
    let js = format!(
        r#"(() => {{ const el = document.querySelector({}); if (!el) throw new Error('Element not found: {}'); el.click(); return true; }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );
    evaluate(session, frame_id, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Element not found") => {
            PatchrightError::ElementNotFound { selector: selector.to_string() }
        }
        other => other,
    })?;
    Ok(())
}

pub async fn fill(session: &Arc<CdpSession>, frame_id: &str, selector: &str, value: &str) -> Result<()> {
    let js = format!(
        r#"(() => {{ const el = document.querySelector({}); if (!el) throw new Error('Element not found: {}'); el.focus(); el.value = {}; el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }})); return true; }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'"),
        serde_json::to_string(value).unwrap()
    );
    evaluate(session, frame_id, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Element not found") => {
            PatchrightError::ElementNotFound { selector: selector.to_string() }
        }
        other => other,
    })?;
    Ok(())
}

pub async fn wait_for_selector(session: &Arc<CdpSession>, frame_id: &str, selector: &str, timeout_ms: u64) -> Result<()> {
    let js = format!(
        r#"(async () => {{ const deadline = Date.now() + {}; while (Date.now() < deadline) {{ if (document.querySelector({})) return true; await new Promise(r => setTimeout(r, 100)); }} throw new Error('Timeout waiting for: {}'); }})()"#,
        timeout_ms,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );
    evaluate(session, frame_id, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Timeout") => {
            PatchrightError::Timeout(format!("wait_for_selector: {selector}"))
        }
        other => other,
    })?;
    Ok(())
}

pub async fn text_content(session: &Arc<CdpSession>, frame_id: &str, selector: &str) -> Result<String> {
    let js = format!(
        r#"(() => {{ const el = document.querySelector({}); if (!el) throw new Error('Element not found: {}'); return el.textContent || ''; }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );
    let value = evaluate(session, frame_id, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Element not found") => {
            PatchrightError::ElementNotFound { selector: selector.to_string() }
        }
        other => other,
    })?;
    Ok(value.as_str().unwrap_or("").to_string())
}
