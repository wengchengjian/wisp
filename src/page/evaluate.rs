use serde_json::{json, Value};
use crate::error::{PatchrightError, Result};
use super::Page;

/// Evaluate JavaScript in an isolated ExecutionContext.
/// Does NOT send Runtime.enable (core anti-detection patch).
pub async fn evaluate(page: &Page, expression: &str) -> Result<Value> {
    // 1. Create isolated world (does NOT require Runtime.enable)
    let world = page.cmd("Page.createIsolatedWorld", json!({
        "frameId": page.frame_id,
        "worldName": "patchright",
        "grantUniveralAccess": true
    })).await?;

    let context_id = world.get("executionContextId")
        .and_then(|c| c.as_u64())
        .ok_or_else(|| PatchrightError::EvalError("no executionContextId".into()))?;

    // 2. Evaluate in that context
    let result = page.cmd("Runtime.evaluate", json!({
        "expression": expression,
        "contextId": context_id,
        "returnByValue": true,
        "awaitPromise": true
    })).await?;

    // 3. Check for exceptions
    if let Some(exception) = result.get("exceptionDetails") {
        let msg = exception.pointer("/exception/description")
            .and_then(|d| d.as_str())
            .or_else(|| exception.get("text").and_then(|t| t.as_str()))
            .unwrap_or("unknown exception");
        return Err(PatchrightError::EvalError(msg.to_string()));
    }

    // 4. Extract value
    Ok(result.pointer("/result/value").cloned().unwrap_or(Value::Null))
}
