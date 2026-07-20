pub mod evaluate;
pub mod navigate;
pub mod screenshot;

use std::sync::Arc;
use serde_json::json;
use crate::cdp::session::CdpSession;
use crate::error::{PatchrightError, Result};

pub struct Page {
    pub(crate) session: Arc<CdpSession>,
    pub(crate) session_id: String,
    pub(crate) frame_id: String,
}

impl Page {
    /// Execute a CDP command on this page's target session.
    pub(crate) async fn cmd(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        self.session.execute_with_session(method, params, Some(&self.session_id)).await
    }

    /// Create a new page via CDP Target domain.
    pub(crate) async fn create(session: Arc<CdpSession>) -> Result<Self> {
        // 1. Create a new target (tab)
        let result = session.execute("Target.createTarget", json!({
            "url": "about:blank"
        })).await?;
        let target_id = result.get("targetId")
            .and_then(|t| t.as_str())
            .ok_or_else(|| PatchrightError::CdpError("no targetId".into()))?
            .to_string();

        // 2. Attach to the target (flatten=true for multiplexed session)
        let result = session.execute("Target.attachToTarget", json!({
            "targetId": target_id,
            "flatten": true
        })).await?;
        let session_id = result.get("sessionId")
            .and_then(|s| s.as_str())
            .ok_or_else(|| PatchrightError::CdpError("no sessionId".into()))?
            .to_string();

        let page = Self { session, session_id, frame_id: String::new() };

        // 3. Enable Page domain on this target
        page.cmd("Page.enable", json!({})).await?;

        // 4. Enable lifecycle events
        page.cmd("Page.setLifecycleEventsEnabled", json!({"enabled": true})).await?;

        // 5. Get frame tree
        let frame_tree = page.cmd("Page.getFrameTree", json!({})).await?;
        let frame_id = frame_tree
            .pointer("/frameTree/frame/id")
            .and_then(|f| f.as_str())
            .unwrap_or("")
            .to_string();

        // 6. Inject stealth scripts
        let stealth_script = crate::patches::stealth::STEALTH_SCRIPT;
        page.cmd("Page.addScriptToEvaluateOnNewDocument", json!({"source": stealth_script})).await?;
        let shadow_dom_script = crate::patches::shadow_dom::SHADOW_DOM_PATCH_SCRIPT;
        page.cmd("Page.addScriptToEvaluateOnNewDocument", json!({"source": shadow_dom_script})).await?;

        // 7. Override User-Agent to remove "HeadlessChrome" marker
        page.cmd("Emulation.setUserAgentOverride", json!({
            "userAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
            "platform": "Win32",
            "userAgentMetadata": {
                "brands": [{"brand": "Chromium", "version": "130"}, {"brand": "Google Chrome", "version": "130"}],
                "fullVersionList": [{"brand": "Chromium", "version": "130.0.6723.92"}, {"brand": "Google Chrome", "version": "130.0.6723.92"}],
                "platform": "Windows",
                "platformVersion": "15.0.0",
                "architecture": "x86",
                "model": "",
                "mobile": false,
                "bitness": "64",
                "wow64": false
            }
        })).await?;

        // NOTE: We do NOT send Runtime.enable!

        Ok(Self { frame_id, ..page })
    }

    pub async fn goto(&self, url: &str) -> Result<()> {
        navigate::goto(self, url).await
    }

    pub async fn reload(&self) -> Result<()> {
        navigate::reload(self).await
    }

    pub async fn evaluate(&self, expression: &str) -> Result<serde_json::Value> {
        evaluate::evaluate(self, expression).await
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
        crate::element::click(self, selector).await
    }

    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> {
        crate::element::fill(self, selector, value).await
    }

    pub async fn wait_for_selector(&self, selector: &str, timeout: Option<std::time::Duration>) -> Result<()> {
        let ms = timeout.unwrap_or(std::time::Duration::from_secs(30)).as_millis() as u64;
        crate::element::wait_for_selector(self, selector, ms).await
    }

    pub async fn text_content(&self, selector: &str) -> Result<String> {
        crate::element::text_content(self, selector).await
    }

    pub async fn screenshot(&self, path: &str) -> Result<()> {
        screenshot::screenshot(self, path).await
    }

    pub async fn screenshot_bytes(&self) -> Result<Vec<u8>> {
        screenshot::screenshot_bytes(self).await
    }
}
