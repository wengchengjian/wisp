//! CF cookie 注入与持久化。

use super::*;

use crate::cookie::CfSession;

impl StealthStrategy {
    pub(super) async fn inject_cf_cookies(&self, page: &mut Page, domain: Option<&str>, url: &str) {
        let Some(domain) = domain else {
            return;
        };
        let Some(session) = self.cf_jar.get_session(domain) else {
            return;
        };
        for cookie in &session.cookies {
            let name = cookie.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let value = cookie.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let cookie_domain = cookie
                .get("domain")
                .and_then(|d| d.as_str())
                .unwrap_or(domain);
            let path = cookie.get("path").and_then(|p| p.as_str()).unwrap_or("/");
            let secure = cookie
                .get("secure")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let http_only = cookie
                .get("httpOnly")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let same_site = cookie
                .get("sameSite")
                .and_then(|s| s.as_str())
                .unwrap_or("Lax");
            let _ = page
                .cmd(
                    "Network.setCookie",
                    serde_json::json!({
                        "name": name,
                        "value": value,
                        "domain": cookie_domain,
                        "path": path,
                        "secure": secure,
                        "httpOnly": http_only,
                        "sameSite": same_site,
                    }),
                )
                .await;
        }
        tracing::info!(
            "BrowserWork[+CF]: {url} 注入 {} 个 CF cookie",
            session.cookies.len()
        );
    }

    pub(super) async fn persist_cf_session(
        &self,
        page: &mut Page,
        domain: Option<&str>,
        url: &str,
    ) {
        let Some(domain) = domain else {
            return;
        };
        let Ok(ua_val) = page.evaluate("navigator.userAgent").await else {
            return;
        };
        let ua_str = ua_val.as_str().unwrap_or("").to_string();
        let Ok(resp) = page.cmd("Network.getCookies", serde_json::json!({})).await else {
            return;
        };
        let Some(cookies) = resp.pointer("/cookies").and_then(|c| c.as_array()) else {
            return;
        };
        let cookies_to_save: Vec<serde_json::Value> = cookies
            .iter()
            .filter(|c| {
                let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
                name.starts_with("cf_") || name.starts_with("__cf")
            })
            .cloned()
            .collect();
        if cookies_to_save.is_empty() {
            return;
        }
        self.cf_jar.insert_session(
            domain.to_string(),
            CfSession {
                cookies: cookies_to_save.clone(),
                ua: ua_str,
                saved_at: chrono::Utc::now().timestamp(),
            },
        );
        tracing::info!(
            "BrowserWork[+CF]: {url} 保存 {} 个 CF cookie",
            cookies_to_save.len()
        );
    }
}
