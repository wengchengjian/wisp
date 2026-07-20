use chromiumoxide::cdp::browser_protocol::page::{CreateIsolatedWorldParams, GetFrameTreeParams};
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use chromiumoxide::Page as CdpPage;
use serde_json::Value;

use crate::error::{PatchrightError, Result};

/// Evaluate JavaScript in an isolated ExecutionContext.
///
/// This is the core anti-detection patch: execute JavaScript WITHOUT sending
/// `Runtime.enable`. Instead, we:
/// 1. Get the page's main frame ID via `Page.getFrameTree`
/// 2. Create an isolated world via `Page.createIsolatedWorld` (does NOT require Runtime.enable)
/// 3. Execute JS in that isolated context via `Runtime.evaluate` with `context_id`
pub async fn evaluate(page: &CdpPage, expression: &str) -> Result<Value> {
    // Step 1: Get frame tree to find main frame ID
    let frame_tree = page
        .execute(GetFrameTreeParams {})
        .await
        .map_err(|e| PatchrightError::EvalError(format!("Failed to get frame tree: {e}")))?;

    let frame_id = frame_tree.result.frame_tree.frame.id;

    // Step 2: Create isolated world (does NOT require Runtime.enable)
    let isolated_world = page
        .execute(
            CreateIsolatedWorldParams::builder()
                .frame_id(frame_id)
                .world_name("patchright")
                .grant_univeral_access(true)
                .build()
                .map_err(|e| PatchrightError::EvalError(format!("Failed to build params: {e}")))?,
        )
        .await
        .map_err(|e| PatchrightError::EvalError(format!("Failed to create isolated world: {e}")))?;

    let context_id = isolated_world.result.execution_context_id;

    // Step 3: Evaluate in that context
    let eval_result = page
        .execute(
            EvaluateParams::builder()
                .expression(expression)
                .context_id(context_id)
                .return_by_value(true)
                .await_promise(true)
                .build()
                .map_err(|e| PatchrightError::EvalError(format!("Failed to build eval params: {e}")))?,
        )
        .await
        .map_err(|e| PatchrightError::EvalError(format!("Evaluation failed: {e}")))?;

    // Check for exceptions
    if let Some(exception) = &eval_result.result.exception_details {
        return Err(PatchrightError::EvalError(format!(
            "JS exception: {exception}"
        )));
    }

    // Extract the value
    Ok(eval_result
        .result
        .result
        .value
        .unwrap_or(Value::Null))
}
