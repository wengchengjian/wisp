//! CDP 事件等待。

use super::{CdpEvent, CdpSession};
use wisp_core::error::{Result, WispError};

impl CdpSession {
    /// Wait for a CDP event matching predicate.
    ///
    /// 匹配成功后更新 consumed_offset，配合 push 端的 drain 防止内存泄漏。
    pub async fn wait_for_event<F>(&self, predicate: F, timeout_ms: u64) -> Result<CdpEvent>
    where
        F: Fn(&CdpEvent) -> bool,
    {
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);
        loop {
            {
                let events = self.events.lock().await;
                if let Some(idx) = events.iter().position(&predicate) {
                    let event = events[idx].clone();
                    // 更新已消费偏移量（idx+1 之前的都算已消费）
                    let mut offset = self.consumed_offset.lock().await;
                    *offset = (*offset).max(idx + 1);
                    return Ok(event);
                }
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(WispError::Timeout("waiting for CDP event".into()));
            }
            tokio::select! {
                _ = self.event_notify.notified() => {}
                _ = tokio::time::sleep(remaining) => {
                    return Err(WispError::Timeout("waiting for CDP event".into()));
                }
            }
        }
    }
}
