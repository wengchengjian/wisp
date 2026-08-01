//! Cookie read/write through the CDP Network domain.

use super::*;

impl Page {
    /// Get all cookies (including httpOnly) via CDP.
    pub async fn cookies(&self) -> Result<Vec<Value>> {
        let resp = self.cmd("Network.getCookies", json!({})).await?;
        Ok(resp
            .get("cookies")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Get a specific cookie value by name (including httpOnly).
    pub async fn get_cookie(&self, name: &str) -> Result<Option<String>> {
        let cookies = self.cookies().await?;
        Ok(cookies
            .iter()
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
}
