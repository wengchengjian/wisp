//! JS evaluation via isolated worlds (no Runtime.enable needed).

use serde_json::{json, Value};
use crate::error::{WispError, Result};
use super::Page;

pub async fn evaluate(page: &Page, expression: &str) -> Result<Value> {
    // Create isolated world (avoids Runtime.enable detection)
    let world = page.cmd("Page.createIsolatedWorld", json!({
        "frameId": page.frame_id,
        "grantUniveralAccess": true,
        "worldName": "patchright"
    })).await?;

    let context_id = world.get("executionContextId").and_then(|id| id.as_u64())
        .ok_or_else(|| WispError::CdpError("no executionContextId".into()))?;

    let result = page.cmd("Runtime.evaluate", json!({
        "expression": expression,
        "contextId": context_id,
        "returnByValue": true,
        "awaitPromise": true
    })).await?;

    if let Some(exception) = result.get("exceptionDetails") {
        let text = exception.get("text").and_then(|t| t.as_str()).unwrap_or("JS error");
        return Err(WispError::EvalError(text.to_string()));
    }

    Ok(result.get("result").and_then(|r| r.get("value")).cloned().unwrap_or(Value::Null))
}
