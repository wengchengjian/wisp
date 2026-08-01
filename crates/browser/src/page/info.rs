//! Read and write page-level document state.

use super::*;

impl Page {
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
        self.evaluate_as_string("document.documentElement.outerHTML")
            .await
    }

    /// Set the page HTML content.
    pub async fn set_content(&self, html: &str) -> Result<()> {
        let escaped = serde_json::to_string(html).expect("serialize &str cannot fail");
        self.evaluate(&format!("document.documentElement.innerHTML = {}", escaped))
            .await?;
        Ok(())
    }
}
