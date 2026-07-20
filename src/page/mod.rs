pub mod evaluate;
pub mod navigate;

use chromiumoxide::Page as CdpPage;

use crate::error::Result;

/// A browser page (tab) with anti-detection patches.
pub struct Page {
    pub(crate) inner: CdpPage,
}

impl Page {
    pub(crate) async fn new(inner: CdpPage) -> Result<Self> {
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
}
