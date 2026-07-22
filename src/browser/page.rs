//! Page operations with anti-detection (isolated worlds, no Runtime.enable).


use std::sync::Arc;
use serde_json::{json, Value};

use crate::browser::cdp::CdpSession;
use crate::error::{WispError, Result};
use base64::Engine;

pub struct Page {
    pub(crate) session: Arc<CdpSession>,
    pub(crate) session_id: String,
    pub(crate) frame_id: String,
}

impl Page {
    /// Execute a raw CDP command on this page's target session.
    pub async fn cmd(&self, method: &str, params: Value) -> Result<Value> {
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

        let page = Self { session, session_id, frame_id };

        // Inject stealth scripts (conditional on headless/headed)
        let stealth_script = if headless {
            crate::browser::patches::HEADLESS_STEALTH_SCRIPT
        } else {
            crate::browser::patches::HEADED_STEALTH_SCRIPT
        };
        page.cmd("Page.addScriptToEvaluateOnNewDocument", json!({"source": stealth_script})).await?;
        // NOTE: shadow_dom patch removed - it overrides Element.prototype.attachShadow
        // which Turnstile detects. We use CDP DOM.getDocument(pierce=true) instead.

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

    // --- Public API: Navigation ---

    pub async fn goto(&self, url: &str) -> Result<()> { do_goto(self, url).await }
    pub async fn reload(&self) -> Result<()> { do_reload(self).await }
    pub async fn go_back(&self) -> Result<()> {
        self.cmd("Page.navigate", json!({"url": "javascript:history.back()"})).await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        Ok(())
    }
    pub async fn go_forward(&self) -> Result<()> {
        self.cmd("Page.navigate", json!({"url": "javascript:history.forward()"})).await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        Ok(())
    }

    // --- Public API: Page Info ---

    /// Get the current page URL.
    pub async fn url(&self) -> Result<String> {
        self.evaluate_as_string("window.location.href").await
    }

    /// Get the page title.
    pub async fn title(&self) -> Result<String> {
        self.evaluate_as_string("document.title").await
    }

    /// Get the full page HTML.
    pub async fn content(&self) -> Result<String> {
        self.evaluate_as_string("document.documentElement.outerHTML").await
    }

    /// Set the page HTML content.
    pub async fn set_content(&self, html: &str) -> Result<()> {
        let escaped = serde_json::to_string(html).unwrap();
        self.evaluate(&format!("document.documentElement.innerHTML = {}", escaped)).await?;
        Ok(())
    }

    // --- Public API: JavaScript ---

    pub async fn evaluate(&self, expression: &str) -> Result<Value> { do_evaluate(self, expression).await }
    pub async fn evaluate_as_string(&self, expression: &str) -> Result<String> {
        let value = self.evaluate(expression).await?;
        Ok(match value { Value::String(s) => s, Value::Null => "null".to_string(), other => other.to_string() })
    }

    // --- Public API: Cookies ---

    /// Get all cookies (including httpOnly) via CDP.
    pub async fn cookies(&self) -> Result<Vec<Value>> {
        let resp = self.cmd("Network.getCookies", json!({})).await?;
        Ok(resp.get("cookies").and_then(|c| c.as_array()).cloned().unwrap_or_default())
    }

    /// Get a specific cookie value by name (including httpOnly).
    pub async fn get_cookie(&self, name: &str) -> Result<Option<String>> {
        let cookies = self.cookies().await?;
        Ok(cookies.iter()
            .find(|c| c.get("name").and_then(|n| n.as_str()) == Some(name))
            .and_then(|c| c.get("value").and_then(|v| v.as_str()))
            .map(|v| v.to_string()))
    }

    /// Add/set cookies.
    pub async fn add_cookies(&self, cookies: &[Value]) -> Result<()> {
        for cookie in cookies {
            self.cmd("Network.setCookie", cookie.clone()).await?;
        }
        Ok(())
    }

    /// Clear all cookies.
    pub async fn clear_cookies(&self) -> Result<()> {
        self.cmd("Network.clearBrowserCookies", json!({})).await?;
        Ok(())
    }

    // --- Public API: Elements ---

    pub async fn click(&self, selector: &str) -> Result<()> { crate::browser::element::click(self, selector).await }
    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> { crate::browser::element::fill(self, selector, value).await }
    pub async fn wait_for_selector(&self, selector: &str, timeout_ms: u64) -> Result<()> { crate::browser::element::wait_for_selector(self, selector, timeout_ms).await }
    pub async fn text_content(&self, selector: &str) -> Result<String> { crate::browser::element::text_content(self, selector).await }

    /// Get inner text of an element.
    pub async fn inner_text(&self, selector: &str) -> Result<String> {
        let js = format!("document.querySelector({})?.innerText || ''", serde_json::to_string(selector).unwrap());
        self.evaluate_as_string(&js).await
    }

    /// Get inner HTML of an element.
    pub async fn inner_html(&self, selector: &str) -> Result<String> {
        let js = format!("document.querySelector({})?.innerHTML || ''", serde_json::to_string(selector).unwrap());
        self.evaluate_as_string(&js).await
    }

    /// Get an attribute value from an element.
    pub async fn get_attribute(&self, selector: &str, attr: &str) -> Result<Option<String>> {
        let js = format!(
            "document.querySelector({})?.getAttribute({})",
            serde_json::to_string(selector).unwrap(),
            serde_json::to_string(attr).unwrap()
        );
        let val = self.evaluate(&js).await?;
        Ok(val.as_str().map(|s| s.to_string()))
    }

    /// Check if an element exists on the page.
    pub async fn query_selector(&self, selector: &str) -> Result<bool> {
        let js = format!("!!document.querySelector({})", serde_json::to_string(selector).unwrap());
        let val = self.evaluate(&js).await?;
        Ok(val.as_bool().unwrap_or(false))
    }

    /// Check if an element is visible.
    pub async fn is_visible(&self, selector: &str) -> Result<bool> {
        let js = format!(r#"(() => {{
            const el = document.querySelector({});
            if (!el) return false;
            const style = window.getComputedStyle(el);
            return style.display !== 'none' && style.visibility !== 'hidden' && el.offsetHeight > 0;
        }})()"#, serde_json::to_string(selector).unwrap());
        let val = self.evaluate(&js).await?;
        Ok(val.as_bool().unwrap_or(false))
    }

    /// Hover over an element.
    pub async fn hover(&self, selector: &str) -> Result<()> {
        let js = format!(r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found');
            const r = el.getBoundingClientRect();
            return {{x: r.x + r.width/2, y: r.y + r.height/2}};
        }})()"#, serde_json::to_string(selector).unwrap());
        let pos = self.evaluate(&js).await?;
        let x = pos.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let y = pos.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
        self.cmd("Input.dispatchMouseEvent", json!({"type": "mouseMoved", "x": x, "y": y})).await?;
        Ok(())
    }

    /// Select an option in a <select> element.
    pub async fn select_option(&self, selector: &str, value: &str) -> Result<()> {
        let js = format!(r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found');
            el.value = {};
            el.dispatchEvent(new Event('change', {{bubbles: true}}));
        }})()"#, serde_json::to_string(selector).unwrap(), serde_json::to_string(value).unwrap());
        self.evaluate(&js).await?;
        Ok(())
    }

    // --- Public API: Input ---

    /// Press a keyboard key (e.g., "Enter", "Tab", "Escape").
    pub async fn press_key(&self, key: &str) -> Result<()> {
        self.cmd("Input.dispatchKeyEvent", json!({"type": "keyDown", "key": key})).await?;
        self.cmd("Input.dispatchKeyEvent", json!({"type": "keyUp", "key": key})).await?;
        Ok(())
    }

    /// Type text character by character (fast, no human simulation).
    pub async fn type_text(&self, text: &str) -> Result<()> {
        for ch in text.chars() {
            self.cmd("Input.dispatchKeyEvent", json!({"type": "keyDown", "text": ch.to_string()})).await?;
            self.cmd("Input.dispatchKeyEvent", json!({"type": "keyUp", "text": ch.to_string()})).await?;
        }
        Ok(())
    }

    // --- Public API: Viewport & Display ---

    /// Set the viewport size.
    pub async fn set_viewport(&self, width: u32, height: u32) -> Result<()> {
        self.cmd("Emulation.setDeviceMetricsOverride", json!({
            "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false
        })).await?;
        Ok(())
    }

    /// Set extra HTTP headers for all requests.
    pub async fn set_extra_http_headers(&self, headers: std::collections::HashMap<String, String>) -> Result<()> {
        self.cmd("Network.setExtraHTTPHeaders", json!({"headers": headers})).await?;
        Ok(())
    }

    // --- Public API: Output ---

    pub async fn screenshot(&self, path: &str) -> Result<()> { do_screenshot(self, path).await }
    pub async fn screenshot_bytes(&self) -> Result<Vec<u8>> { do_screenshot_bytes(self).await }

    /// Generate a PDF (headless only).
    pub async fn pdf(&self, path: &str) -> Result<()> {
        let result = self.cmd("Page.printToPDF", json!({"printBackground": true})).await?;
        let data = result.get("data").and_then(|d| d.as_str())
            .ok_or_else(|| WispError::CdpError("no PDF data".into()))?;
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(data)
            .map_err(|e| WispError::CdpError(format!("decode PDF: {e}")))?;
        tokio::fs::write(path, &bytes).await
            .map_err(|e| WispError::CdpError(format!("write PDF: {e}")))?;
        Ok(())
    }

    // --- Public API: Wait ---

    /// Wait for a specific URL pattern (substring match).
    pub async fn wait_for_url(&self, url_pattern: &str, timeout_ms: u64) -> Result<()> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let current = self.url().await?;
            if current.contains(url_pattern) { return Ok(()); }
            if tokio::time::Instant::now() > deadline {
                return Err(WispError::Timeout(format!("wait_for_url: {url_pattern}")));
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// Wait for the page to reach a specific ready state.
    pub async fn wait_for_load_state(&self, timeout_ms: u64) -> Result<()> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let state = self.evaluate_as_string("document.readyState").await?;
            if state == "complete" { return Ok(()); }
            if tokio::time::Instant::now() > deadline {
                return Err(WispError::Timeout("wait_for_load_state".into()));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

// === evaluate (inlined) ===
// JS evaluation via isolated worlds (no Runtime.enable needed).



pub async fn do_evaluate(page: &Page, expression: &str) -> Result<Value> {
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

// === navigate (inlined) ===


pub async fn do_goto(page: &Page, url: &str) -> Result<()> {
    page.cmd("Page.navigate", json!({ "url": url })).await?;
    // Wait for page load using lifecycle event or timeout
    wait_for_load(page).await
}

pub async fn do_reload(page: &Page) -> Result<()> {
    page.cmd("Page.reload", json!({})).await?;
    wait_for_load(page).await
}

async fn wait_for_load(page: &Page) -> Result<()> {
    // Try to wait for load event, but don't fail if it times out
    // (some pages like about:blank don't fire load events the same way)
    let sid = page.session_id.clone();
    let result = page.session.wait_for_event(
        move |e| {
            if e.method == "Page.loadEventFired" {
                return e.session_id.as_deref() == Some(sid.as_str()) || e.session_id.is_none();
            }
            // Also accept lifecycleEvent with name "load"
            if e.method == "Page.lifecycleEvent" {
                if e.params.get("name").and_then(|n| n.as_str()) == Some("load") {
                    return e.session_id.as_deref() == Some(sid.as_str()) || e.session_id.is_none();
                }
            }
            false
        },
        15000,
    ).await;

    // If event wait fails, fall back to a short delay
    if result.is_err() {
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }
    Ok(())
}

// === screenshot (inlined) ===


pub async fn do_screenshot(page: &Page, path: &str) -> Result<()> {
    let bytes = do_screenshot_bytes(page).await?;
    tokio::fs::write(path, &bytes).await
        .map_err(|e| WispError::CdpError(format!("write: {e}")))?;
    Ok(())
}

pub async fn do_screenshot_bytes(page: &Page) -> Result<Vec<u8>> {
    let result = page.cmd("Page.captureScreenshot", json!({"format": "png"})).await?;
    let data = result.get("data").and_then(|d| d.as_str())
        .ok_or_else(|| WispError::CdpError("no screenshot data".into()))?;
    base64::engine::general_purpose::STANDARD.decode(data)
        .map_err(|e| WispError::CdpError(format!("decode: {e}")))
}
