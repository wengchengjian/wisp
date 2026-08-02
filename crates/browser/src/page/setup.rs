//! Page target creation, CDP session setup and stealth bootstrap.

use super::*;
use serde_json::json;
use wisp_core::error::{BrowserError, WispError};

impl Page {
    /// 导航后刷新 frame_id（解决跨域导航后 isolated world context 失效问题）。
    pub(crate) async fn refresh_frame_id(&mut self) {
        if let Ok(frame_tree) = self
            .session
            .execute_with_session("Page.getFrameTree", json!({}), Some(&self.session_id))
            .await
            && let Some(id) = frame_tree
                .get("frameTree")
                .and_then(|ft| ft.get("frame"))
                .and_then(|f| f.get("id"))
                .and_then(|id| id.as_str())
        {
            self.frame_id = id.to_string();
        }
    }

    async fn create_page_target(session: &Arc<CdpSession>) -> Result<String> {
        let result = session
            .execute("Target.createTarget", json!({ "url": "about:blank" }))
            .await?;
        result
            .get("targetId")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| WispError::Browser(BrowserError::CdpConnection("no targetId".into())))
    }

    async fn attach_page_target(session: &Arc<CdpSession>, target_id: &str) -> Result<String> {
        let result = session
            .execute(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;
        result
            .get("sessionId")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| WispError::Browser(BrowserError::CdpConnection("no sessionId".into())))
    }

    async fn init_page_session(session: &Arc<CdpSession>, session_id: &str) -> Result<String> {
        session
            .execute_with_session("Page.enable", json!({}), Some(session_id))
            .await?;
        let frame_tree = session
            .execute_with_session("Page.getFrameTree", json!({}), Some(session_id))
            .await?;
        let frame_id = frame_tree
            .get("frameTree")
            .and_then(|ft| ft.get("frame"))
            .and_then(|f| f.get("id"))
            .and_then(|id| id.as_str())
            .ok_or_else(|| WispError::Browser(BrowserError::CdpConnection("no frame id".into())))?
            .to_string();
        let _ = session
            .execute_with_session("Log.enable", json!({}), Some(session_id))
            .await;
        session
            .execute_with_session(
                "Page.setLifecycleEventsEnabled",
                json!({ "enabled": true }),
                Some(session_id),
            )
            .await?;
        Ok(frame_id)
    }

    async fn inject_page_stealth(page: &Page, headless: bool) -> Result<()> {
        let stealth_script = if headless {
            crate::patches::HEADLESS_STEALTH_SCRIPT
        } else {
            crate::patches::HEADED_STEALTH_SCRIPT
        };
        page.cmd(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": stealth_script }),
        )
        .await?;
        Ok(())
    }

    async fn override_headless_ua(page: &Page) -> Result<()> {
        let version_info = page
            .session
            .execute("Browser.getVersion", json!({}))
            .await?;
        let product = version_info
            .get("product")
            .and_then(|p| p.as_str())
            .unwrap_or("Chrome/130.0.0.0");
        let version = product.strip_prefix("Chrome/").unwrap_or("130.0.0.0");
        let major = version.split('.').next().unwrap_or("130");
        let ua = format!(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{} Safari/537.36",
            version
        );
        page.cmd(
            "Emulation.setUserAgentOverride",
            json!({
                "userAgent": ua,
                "platform": "Win32",
                "userAgentMetadata": {
                    "brands": [
                        { "brand": "Chromium", "version": major },
                        { "brand": "Google Chrome", "version": major }
                    ],
                    "fullVersionList": [
                        { "brand": "Chromium", "version": version },
                        { "brand": "Google Chrome", "version": version }
                    ],
                    "platform": "Windows", "platformVersion": "15.0.0",
                    "architecture": "x86", "model": "", "mobile": false, "bitness": "64", "wow64": false
                }
            }),
        )
        .await?;
        Ok(())
    }

    /// Create a new page via CDP Target domain.
    pub(crate) async fn create(session: Arc<CdpSession>, headless: bool) -> Result<Self> {
        let target_id = Self::create_page_target(&session).await?;
        let session_id = Self::attach_page_target(&session, &target_id).await?;
        let frame_id = Self::init_page_session(&session, &session_id).await?;
        let page = Self {
            session,
            session_id,
            frame_id,
            target_id: Some(target_id),
        };
        Self::inject_page_stealth(&page, headless).await?;
        if headless {
            Self::override_headless_ua(&page).await?;
        }
        Ok(page)
    }
}
