pub mod evaluate;
pub mod navigate;
pub mod screenshot;

use chromiumoxide::Page as CdpPage;

use crate::error::Result;

/// A browser page (tab) with anti-detection patches.
pub struct Page {
    pub(crate) inner: CdpPage,
}

impl Page {
    pub(crate) async fn new(inner: CdpPage) -> Result<Self> {
        // Inject shadow DOM patch before any page content loads
        crate::patches::shadow_dom::inject(&inner).await?;
        Ok(Self { inner })
    }

    /// Navigate to a URL and wait for load.
    pub async fn goto(&self, url: &str) -> Result<()> {
        navigate::goto(&self.inner, url).await
    }

    /// Reload the current page.
    pub async fn reload(&self) -> Result<()> {
        navigate::reload(&self.inner).await
    }

    /// Evaluate JavaScript in an isolated ExecutionContext.
    ///
    /// Does NOT send `Runtime.enable` (core anti-detection patch).
    pub async fn evaluate(&self, expression: &str) -> Result<serde_json::Value> {
        evaluate::evaluate(&self.inner, expression).await
    }

    /// Evaluate JavaScript and return result as String.
    pub async fn evaluate_as_string(&self, expression: &str) -> Result<String> {
        let value = evaluate::evaluate(&self.inner, expression).await?;
        Ok(match value {
            serde_json::Value::String(s) => s,
            serde_json::Value::Null => "null".to_string(),
            other => other.to_string(),
        })
    }

    /// Click an element matching the CSS selector.
    pub async fn click(&self, selector: &str) -> Result<()> {
        crate::element::click(&self.inner, selector).await
    }

    /// Type text into an input element.
    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> {
        crate::element::fill(&self.inner, selector, value).await
    }

    /// Wait for an element to appear in the DOM.
    pub async fn wait_for_selector(&self, selector: &str, timeout: Option<std::time::Duration>) -> Result<()> {
        let ms = timeout.unwrap_or(std::time::Duration::from_secs(30)).as_millis() as u64;
        crate::element::wait_for_selector(&self.inner, selector, ms).await
    }

    /// Get text content of an element.
    pub async fn text_content(&self, selector: &str) -> Result<String> {
        crate::element::text_content(&self.inner, selector).await
    }

    /// Capture a full-page screenshot and save to file.
    pub async fn screenshot(&self, path: &str) -> Result<()> {
        screenshot::screenshot(&self.inner, path).await
    }

    /// Capture a screenshot and return raw PNG bytes.
    pub async fn screenshot_bytes(&self) -> Result<Vec<u8>> {
        screenshot::screenshot_bytes(&self.inner).await
    }
}
