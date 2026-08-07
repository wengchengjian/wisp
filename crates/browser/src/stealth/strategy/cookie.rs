//! CF cookie 注入与持久化。

use super::*;

use crate::cookie::BrowserCookieJar;

impl StealthStrategy {
    pub(super) async fn inject_cf_cookies(&self, page: &mut Page, domain: Option<&str>, url: &str) {
        let Some(domain) = domain else {
            return;
        };
        let Ok(parsed) = url::Url::parse(url) else {
            return;
        };
        let cookies = self.cookie_jar.get(&parsed).await;
        if cookies.is_empty() {
            return;
        }
        let browser_jar = BrowserCookieJar::new_for_target(
            Arc::clone(page.session()),
            page.session_id().to_string(),
        );
        for cookie in cookies {
            browser_jar.set(cookie).await;
        }
        tracing::info!("BrowserWork[+CF]: {url} 注入 CF cookie（domain={domain}）");
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
        let Ok(parsed) = url::Url::parse(url) else {
            return;
        };
        let Ok(ua_val) = page.evaluate("navigator.userAgent").await else {
            return;
        };
        let ua_str = ua_val.as_str().unwrap_or("").to_string();
        let browser_jar = BrowserCookieJar::new_for_target(
            Arc::clone(page.session()),
            page.session_id().to_string(),
        );
        let cookies = browser_jar.get(&parsed).await;
        // 保存浏览器的全部 cookie（而非仅 cf_*/__cf），否则会漏掉 CF 必带的
        // `_cfuvid`（Cloudflare Unique Visitor ID，不以 cf_/__cf 开头），
        // 导致 HTTP 快速路径复用 cookie 时被 CF 拒（403）。
        // 旧版 banzhu（CfManager）用 `all_cookies`（含 _cfuvid）也证明了需保存完整 cookie。
        if cookies.is_empty() {
            return;
        }
        self.cookie_jar.set_batch(cookies).await;
        self.cookie_jar.set_session_ua(domain, Some(&ua_str)).await;
        tracing::info!("BrowserWork[+CF]: {url} 保存 CF cookie（domain={domain}）");
    }
}
