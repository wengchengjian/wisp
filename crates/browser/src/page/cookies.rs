//! Cookie read/write through the CDP Network domain.

use super::*;

fn cookie_to_string(c: &Value) -> Option<String> {
    let name = c.get("name")?.as_str()?;
    let value = c.get("value")?.as_str()?;
    Some(format!("{name}={value}"))
}

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

    /// Get cookies scoped to one URL as `name=value` strings (including httpOnly).
    pub async fn cookie_strings(&self, url: &str) -> Result<Vec<String>> {
        let resp = self
            .cmd("Network.getCookies", json!({ "urls": [url] }))
            .await?;
        Ok(resp
            .get("cookies")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(cookie_to_string)
            .collect())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_to_string_formats_name_value() {
        let c = serde_json::json!({ "name": "sid", "value": "abc", "httpOnly": true });
        assert_eq!(cookie_to_string(&c).as_deref(), Some("sid=abc"));
    }

    #[test]
    fn cookie_to_string_skips_missing_parts() {
        assert!(cookie_to_string(&serde_json::json!({ "name": "sid" })).is_none());
        assert!(cookie_to_string(&serde_json::json!({ "value": "abc" })).is_none());
    }
}
