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
        // 捕获浏览器真实 sec-ch-ua：从 navigator.userAgentData.brands 构造，
        // 与浏览器实际请求头格式一致（如 `"Not/A)Brand";v="99", "Chromium";v="148"`）。
        // 不能靠 FetchClient 手动构造（不同版本 brand 顺序/GREASE 值不同），
        // 不一致会被 CF 判定会话无效（403）。
        let sec_ch_ua = capture_sec_ch_ua(page).await;
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
        self.cookie_jar
            .set_session_sec_ch_ua(domain, sec_ch_ua.as_deref())
            .await;
        tracing::info!("BrowserWork[+CF]: {url} 保存 CF cookie（domain={domain}）");
    }
}

/// 从浏览器 `navigator.userAgentData.brands` 构造真实的 sec-ch-ua 头。
///
/// 格式与浏览器实际请求头一致：`"{brand}";v="{version}", ...`（按浏览器返回顺序）。
/// 读取失败或 brands 为空时返回 `None`。
async fn capture_sec_ch_ua(page: &mut Page) -> Option<String> {
    let js = r#"JSON.stringify(navigator.userAgentData && navigator.userAgentData.brands || [])"#;
    let val = page.evaluate(js).await.ok()?;
    let brands: Vec<serde_json::Value> = serde_json::from_str(val.as_str()?).ok()?;
    if brands.is_empty() {
        return None;
    }
    let parts: Vec<String> = brands
        .iter()
        .filter_map(|b| {
            let brand = b.get("brand")?.as_str()?;
            let version = b.get("version")?.as_str()?;
            Some(format!(r#""{brand}";v="{version}""#))
        })
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}
