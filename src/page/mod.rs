//! Page operations with anti-detection (isolated worlds, no Runtime.enable).

pub mod evaluate;
pub mod navigate;
pub mod screenshot;

use std::sync::Arc;
use serde_json::{json, Value};

use crate::cdp::CdpSession;
use crate::error::{WispError, Result};

pub struct Page {
    pub(crate) session: Arc<CdpSession>,
    pub(crate) session_id: String,
    pub(crate) frame_id: String,
    pub(crate) headless: bool,
}

impl Page {
    pub(crate) async fn cmd(&self, method: &str, params: Value) -> Result<Value> {
        self.session.execute_with_session(method, params, Some(&self.session_id)).await
    }

    /// Create a new page via CDP Target domain.
    pub(crate) async fn create(session: Arc<CdpSession>, headless: bool) -> Result<Self> {
        // Create target
        let result = session.execute("Target.createTarget", json!({"url": "about:blank"})).await?;
        let target_id = result.get("targetId").and_then(|t| t.as_str())
            .ok_or_else(|| WispError::CdpError("no targetId".into()))?.to_string();

        // Attach to target
        let result = session.execute("Target.attachToTarget", json!({"targetId": target_id, "flatten": true})).await?;
        let session_id = result.get("sessionId").and_then(|s| s.as_str())
            .ok_or_else(|| WispError::CdpError("no sessionId".into()))?.to_string();

        // Page init sequence (matches patchright):
        // Page.enable -> Page.getFrameTree -> Log.enable -> Page.setLifecycleEventsEnabled
        // NEVER send Runtime.enable or Console.enable!
        session.execute_with_session("Page.enable", json!({}), Some(&session_id)).await?;
        let frame_tree = session.execute_with_session("Page.getFrameTree", json!({}), Some(&session_id)).await?;
        let frame_id = frame_tree.get("frameTree").and_then(|ft| ft.get("frame")).and_then(|f| f.get("id")).and_then(|id| id.as_str())
            .ok_or_else(|| WispError::CdpError("no frame id".into()))?.to_string();
        let _ = session.execute_with_session("Log.enable", json!({}), Some(&session_id)).await;
        session.execute_with_session("Page.setLifecycleEventsEnabled", json!({"enabled": true}), Some(&session_id)).await?;

        let page = Self { session, session_id, frame_id, headless };

        // Inject stealth scripts (conditional on headless/headed)
        let stealth_script = if headless {
            crate::patches::stealth::HEADLESS_STEALTH_SCRIPT
        } else {
            crate::patches::stealth::HEADED_STEALTH_SCRIPT
        };
        page.cmd("Page.addScriptToEvaluateOnNewDocument", json!({"source": stealth_script})).await?;
        page.cmd("Page.addScriptToEvaluateOnNewDocument", json!({"source": crate::patches::shadow_dom::SHADOW_DOM_PATCH_SCRIPT})).await?;

        // Override User-Agent ONLY in headless mode (headed UA is already clean)
        if headless {
            let version_info = page.session.execute("Browser.getVersion", json!({})).await?;
            let product = version_info.get("product").and_then(|p| p.as_str()).unwrap_or("Chrome/130.0.0.0");
            let version = product.strip_prefix("Chrome/").unwrap_or("130.0.0.0");
            let major = version.split('.').next().unwrap_or("130");
            let ua = format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{} Safari/537.36", version);
            page.cmd("Emulation.setUserAgentOverride", json!({
                "userAgent": ua,
                "platform": "Win32",
                "userAgentMetadata": {
                    "brands": [{"brand": "Chromium", "version": major}, {"brand": "Google Chrome", "version": major}],
                    "fullVersionList": [{"brand": "Chromium", "version": version}, {"brand": "Google Chrome", "version": version}],
                    "platform": "Windows", "platformVersion": "15.0.0",
                    "architecture": "x86", "model": "", "mobile": false, "bitness": "64", "wow64": false
                }
            })).await?;
        }

        Ok(page)
    }

    // --- Public API ---

    pub async fn goto(&self, url: &str) -> Result<()> { navigate::goto(self, url).await }
    pub async fn reload(&self) -> Result<()> { navigate::reload(self).await }
    pub async fn evaluate(&self, expression: &str) -> Result<Value> { evaluate::evaluate(self, expression).await }
    pub async fn evaluate_as_string(&self, expression: &str) -> Result<String> {
        let value = self.evaluate(expression).await?;
        Ok(match value { Value::String(s) => s, Value::Null => "null".to_string(), other => other.to_string() })
    }
    pub async fn click(&self, selector: &str) -> Result<()> { crate::element::click(self, selector).await }
    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> { crate::element::fill(self, selector, value).await }
    pub async fn wait_for_selector(&self, selector: &str, timeout_ms: u64) -> Result<()> { crate::element::wait_for_selector(self, selector, timeout_ms).await }
    pub async fn text_content(&self, selector: &str) -> Result<String> { crate::element::text_content(self, selector).await }
    pub async fn screenshot(&self, path: &str) -> Result<()> { screenshot::screenshot(self, path).await }
}
