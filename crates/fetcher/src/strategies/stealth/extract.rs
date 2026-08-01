//! 页面等待与响应提取。

use super::*;

use crate::strategy::extract_browser_response;

impl StealthStrategy {
    pub(super) async fn wait_and_extract(
        &self,
        page: &mut Page,
        req: &Request,
        nav_status: u16,
    ) -> Result<Response> {
        if let Some(ref selector) = self.wait_for {
            page.wait_for_selector(selector, self.timeout.as_millis() as u64)
                .await?;
        }
        if self.extra_wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.extra_wait_ms)).await;
        }
        let url = &req.url;
        tracing::debug!("BrowserWork[+CF]: {url} 提取响应");
        let resp = extract_browser_response(page, req, nav_status).await?;
        tracing::info!("BrowserWork[+CF]: {url} 完成 ({} bytes)", resp.body.len());
        Ok(resp)
    }
}
