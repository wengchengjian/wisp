//! Navigation, reload and load-state waiting.

use super::*;
use crate::cdp::CdpEvent;
use wisp_core::error::WispError;

impl Page {
    /// 导航到指定 URL。
    pub async fn goto(&mut self, url: &str) -> Result<()> {
        do_goto(self, url).await
    }
    /// 重新加载当前页面。
    pub async fn reload(&self) -> Result<()> {
        do_reload(self).await
    }
    /// 后退（历史记录）。
    pub async fn go_back(&self) -> Result<()> {
        self.cmd(
            "Page.navigate",
            json!({ "url": "javascript:history.back()" }),
        )
        .await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        Ok(())
    }
    /// 前进（历史记录）。
    pub async fn go_forward(&self) -> Result<()> {
        self.cmd(
            "Page.navigate",
            json!({ "url": "javascript:history.forward()" }),
        )
        .await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        Ok(())
    }

    /// Wait for a specific URL pattern (substring match).
    pub async fn wait_for_url(&self, url_pattern: &str, timeout_ms: u64) -> Result<()> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let current = self.url().await?;
            if current.contains(url_pattern) {
                return Ok(());
            }
            if tokio::time::Instant::now() > deadline {
                return Err(WispError::Timeout(format!("wait_for_url: {url_pattern}")));
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// Wait for the page to reach a specific ready state.
    pub async fn wait_for_load_state(&self, timeout_ms: u64) -> Result<()> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let state = self.evaluate_as_string("document.readyState").await?;
            if state == "complete" {
                return Ok(());
            }
            if tokio::time::Instant::now() > deadline {
                return Err(WispError::Timeout("wait_for_load_state".into()));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

fn is_load_event(event: &CdpEvent) -> bool {
    event.method == "Page.loadEventFired"
        || (event.method == "Page.lifecycleEvent"
            && event.params.get("name").and_then(|n| n.as_str()) == Some("load"))
}

/// 导航到 URL 并等待页面加载完成。
pub async fn do_goto(page: &mut Page, url: &str) -> Result<()> {
    page.cmd("Page.navigate", json!({ "url": url })).await?;
    // Wait for page load using lifecycle event or timeout
    wait_for_load(page).await?;
    // 导航后刷新 frame_id，避免跨域导航后 isolated world 创建失败
    page.refresh_frame_id().await;
    Ok(())
}

/// 重新加载页面并等待加载完成。
pub async fn do_reload(page: &Page) -> Result<()> {
    page.cmd("Page.reload", json!({})).await?;
    wait_for_load(page).await
}

async fn wait_for_load(page: &Page) -> Result<()> {
    let sid = page.session_id.clone();
    let mut rx = page.session.subscribe_events();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(15000);
    let start = std::time::Instant::now();
    let mut found = false;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(event)) => {
                let match_session =
                    event.session_id.as_deref() == Some(sid.as_str()) || event.session_id.is_none();
                if match_session && is_load_event(&event) {
                    found = true;
                    break;
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }
    tracing::debug!(
        "wait_for_load: 耗时 {}ms, 结果={}",
        start.elapsed().as_millis(),
        if found {
            "Ok(找到新事件)"
        } else {
            "超时(15s)"
        }
    );
    if !found {
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }
    Ok(())
}
