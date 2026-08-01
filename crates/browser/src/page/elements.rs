//! Element query, interaction and viewport operations.

use super::*;

impl Page {
    /// 点击匹配选择器的元素。
    pub async fn click(&self, selector: &str) -> Result<()> {
        crate::element::click(self, selector).await
    }
    /// 向匹配选择器的输入框填充文本。
    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> {
        crate::element::fill(self, selector, value).await
    }
    /// 等待匹配选择器的元素出现。
    pub async fn wait_for_selector(&self, selector: &str, timeout_ms: u64) -> Result<()> {
        crate::element::wait_for_selector(self, selector, timeout_ms).await
    }
    /// 获取匹配选择器元素的文本内容。
    pub async fn text_content(&self, selector: &str) -> Result<String> {
        crate::element::text_content(self, selector).await
    }

    /// Get inner text of an element.
    pub async fn inner_text(&self, selector: &str) -> Result<String> {
        let js = format!(
            "document.querySelector({})?.innerText || ''",
            serde_json::to_string(selector).expect("serialize &str cannot fail")
        );
        self.evaluate_as_string(&js).await
    }

    /// Get inner HTML of an element.
    pub async fn inner_html(&self, selector: &str) -> Result<String> {
        let js = format!(
            "document.querySelector({})?.innerHTML || ''",
            serde_json::to_string(selector).expect("serialize &str cannot fail")
        );
        self.evaluate_as_string(&js).await
    }

    /// Get an attribute value from an element.
    pub async fn get_attribute(&self, selector: &str, attr: &str) -> Result<Option<String>> {
        let js = format!(
            "document.querySelector({})?.getAttribute({})",
            serde_json::to_string(selector).expect("serialize &str cannot fail"),
            serde_json::to_string(attr).expect("serialize &str cannot fail")
        );
        let val = self.evaluate(&js).await?;
        Ok(val.as_str().map(|s| s.to_string()))
    }

    /// Check if an element exists on the page.
    pub async fn query_selector(&self, selector: &str) -> Result<bool> {
        let js = format!(
            "!!document.querySelector({})",
            serde_json::to_string(selector).expect("serialize &str cannot fail")
        );
        let val = self.evaluate(&js).await?;
        Ok(val.as_bool().unwrap_or(false))
    }

    /// Check if an element is visible.
    pub async fn is_visible(&self, selector: &str) -> Result<bool> {
        let js = format!(
            r#"(() => {{
            const el = document.querySelector({});
            if (!el) return false;
            const style = window.getComputedStyle(el);
            return style.display !== 'none' && style.visibility !== 'hidden' && el.offsetHeight > 0;
        }})()"#,
            serde_json::to_string(selector).expect("serialize &str cannot fail")
        );
        let val = self.evaluate(&js).await?;
        Ok(val.as_bool().unwrap_or(false))
    }

    /// Hover over an element.
    pub async fn hover(&self, selector: &str) -> Result<()> {
        let js = format!(
            r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found');
            const r = el.getBoundingClientRect();
            return {{x: r.x + r.width/2, y: r.y + r.height/2}};
        }})()"#,
            serde_json::to_string(selector).expect("serialize &str cannot fail")
        );
        let pos = self.evaluate(&js).await?;
        let x = pos.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let y = pos.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
        self.cmd(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": x, "y": y }),
        )
        .await?;
        Ok(())
    }

    /// Select an option in a <select> element.
    pub async fn select_option(&self, selector: &str, value: &str) -> Result<()> {
        let js = format!(
            r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found');
            el.value = {};
            el.dispatchEvent(new Event('change', {{bubbles: true}}));
        }})()"#,
            serde_json::to_string(selector).expect("serialize &str cannot fail"),
            serde_json::to_string(value).expect("serialize &str cannot fail")
        );
        self.evaluate(&js).await?;
        Ok(())
    }

    /// Press a keyboard key (e.g., "Enter", "Tab", "Escape").
    pub async fn press_key(&self, key: &str) -> Result<()> {
        self.cmd(
            "Input.dispatchKeyEvent",
            json!({ "type": "keyDown", "key": key }),
        )
        .await?;
        self.cmd(
            "Input.dispatchKeyEvent",
            json!({ "type": "keyUp", "key": key }),
        )
        .await?;
        Ok(())
    }

    /// Type text character by character (fast, no human simulation).
    pub async fn type_text(&self, text: &str) -> Result<()> {
        for ch in text.chars() {
            self.cmd(
                "Input.dispatchKeyEvent",
                json!({ "type": "keyDown", "text": ch.to_string() }),
            )
            .await?;
            self.cmd(
                "Input.dispatchKeyEvent",
                json!({ "type": "keyUp", "text": ch.to_string() }),
            )
            .await?;
        }
        Ok(())
    }

    /// Set the viewport size.
    pub async fn set_viewport(&self, width: u32, height: u32) -> Result<()> {
        self.cmd(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false
            }),
        )
        .await?;
        Ok(())
    }

    /// Set extra HTTP headers for all requests.
    pub async fn set_extra_http_headers(
        &self,
        headers: std::collections::HashMap<String, String>,
    ) -> Result<()> {
        self.cmd("Network.setExtraHTTPHeaders", json!({ "headers": headers }))
            .await?;
        Ok(())
    }
}
