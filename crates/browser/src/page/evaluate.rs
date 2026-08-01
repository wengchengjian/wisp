//! JavaScript evaluation in an isolated world.

use super::*;
use wisp_core::error::{BrowserError, WispError};

impl Page {
    /// 执行 JavaScript 表达式，返回 JSON 值。
    pub async fn evaluate(&self, expression: &str) -> Result<Value> {
        do_evaluate(self, expression).await
    }
    /// 执行 JavaScript 表达式，返回字符串结果。
    pub async fn evaluate_as_string(&self, expression: &str) -> Result<String> {
        let value = self.evaluate(expression).await?;
        Ok(match value {
            Value::String(s) => s,
            Value::Null => "null".to_string(),
            other => other.to_string(),
        })
    }
}

/// 在隔离世界中执行 JS（无需 Runtime.enable，避免被检测）。
pub async fn do_evaluate(page: &Page, expression: &str) -> Result<Value> {
    // Create isolated world (avoids Runtime.enable detection)
    let world = page
        .cmd(
            "Page.createIsolatedWorld",
            json!({
                "frameId": page.frame_id,
                "grantUniveralAccess": true,
                "worldName": "patchright"
            }),
        )
        .await?;

    let context_id = world
        .get("executionContextId")
        .and_then(|id| id.as_u64())
        .ok_or_else(|| {
            WispError::Browser(BrowserError::CdpConnection("no executionContextId".into()))
        })?;

    let result = page
        .cmd(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "contextId": context_id,
                "returnByValue": true,
                "awaitPromise": true
            }),
        )
        .await?;

    if let Some(exception) = result.get("exceptionDetails") {
        let text = exception
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("JS error");
        return Err(WispError::Browser(BrowserError::EvalError(
            text.to_string(),
        )));
    }

    Ok(result
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(Value::Null))
}
