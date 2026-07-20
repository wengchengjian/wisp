pub mod evaluate;
pub mod navigate;
pub mod screenshot;

use std::sync::Arc;
use serde_json::json;
use crate::cdp::session::CdpSession;
use crate::error::{PatchrightError, Result};

pub struct Page {
    session: Arc<CdpSession>,
    frame_id: String,
}

impl Page {
    /// Create a new page via CDP Target domain.
    /// Flow: createTarget → attachToTarget → Page.enable → inject stealth scripts
    /// CRITICAL: Does NOT send Runtime.enable
    pub(crate) async fn create(session: Arc<CdpSession>) -> Result<Self> {
        // 1. Create a new target (tab)
        let result = session.execute("Target.createTarget", json!({
            "url": "about:blank"
        })).await?;
        let target_id = result.get("targetId")
            .and_then(|t| t.as_str())
            .ok_or_else(|| PatchrightError::CdpError("no targetId in response".into()))?
            .to_string();

        // 2. Attach to the target
        let result = session.execute("Target.attachToTarget", json!({
            "targetId": target_id,
            "flatten": true
        })).await?;
        let _session_id = result.get("sessionId")
            .and_then(|s| s.as_str())
            .unwrap_or("");

        // 3. Enable Page domain
        session.execute("Page.enable", json!({})).await?;

        // 4. Enable lifecycle events (for navigation waiting)
        session.execute("Page.setLifecycleEventsEnabled", json!({
            "enabled": true
        })).await?;

        // 5. Get frame tree to find main frame ID
        let frame_tree = session.execute("Page.getFrameTree", json!({})).await?;
        let frame_id = frame_tree
            .pointer("/frameTree/frame/id")
            .and_then(|f| f.as_str())
            .unwrap_or("")
            .to_string();

        // 6. Inject stealth scripts (addScriptToEvaluateOnNewDocument)
        // These run before any page scripts on every new document
        let stealth_script = crate::patches::stealth::STEALTH_SCRIPT;
        session.execute("Page.addScriptToEvaluateOnNewDocument", json!({
            "source": stealth_script
        })).await?;

        let shadow_dom_script = crate::patches::shadow_dom::SHADOW_DOM_PATCH_SCRIPT;
        session.execute("Page.addScriptToEvaluateOnNewDocument", json!({
            "source": shadow_dom_script
        })).await?;

        // NOTE: We do NOT send Runtime.enable here!

        Ok(Self { session, frame_id })
    }

    pub async fn goto(&self, url: &str) -> Result<()> {
        navigate::goto(&self.session, url).await
    }

    pub async fn reload(&self) -> Result<()> {
        navigate::reload(&self.session).await
    }

    pub async fn evaluate(&self, expression: &str) -> Result<serde_json::Value> {
        evaluate::evaluate(&self.session, &self.frame_id, expression).await
    }

    pub async fn evaluate_as_string(&self, expression: &str) -> Result<String> {
        let value = self.evaluate(expression).await?;
        Ok(match value {
            serde_json::Value::String(s) => s,
            serde_json::Value::Null => "null".to_string(),
            other => other.to_string(),
        })
    }

    pub async fn click(&self, selector: &str) -> Result<()> {
        crate::element::click(&self.session, &self.frame_id, selector).await
    }

    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> {
        crate::element::fill(&self.session, &self.frame_id, selector, value).await
    }

    pub async fn wait_for_selector(&self, selector: &str, timeout: Option<std::time::Duration>) -> Result<()> {
        let ms = timeout.unwrap_or(std::time::Duration::from_secs(30)).as_millis() as u64;
        crate::element::wait_for_selector(&self.session, &self.frame_id, selector, ms).await
    }

    pub async fn text_content(&self, selector: &str) -> Result<String> {
        crate::element::text_content(&self.session, &self.frame_id, selector).await
    }

    pub async fn screenshot(&self, path: &str) -> Result<()> {
        screenshot::screenshot(&self.session, path).await
    }

    pub async fn screenshot_bytes(&self) -> Result<Vec<u8>> {
        screenshot::screenshot_bytes(&self.session).await
    }
}
