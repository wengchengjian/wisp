//! Page operations with anti-detection (isolated worlds, no Runtime.enable).

mod cookies;
mod elements;
mod evaluate;
mod info;
mod navigation;
mod output;
mod setup;

pub use evaluate::do_evaluate;
pub use navigation::{do_goto, do_reload};
pub use output::{do_screenshot, do_screenshot_bytes};

use serde_json::{json, Value};
use std::sync::Arc;

use crate::cdp::CdpSession;
use wisp_core::error::Result;

/// 浏览器页面（tab）：封装导航、JS 执行、截图等操作。
///
/// 通过 [`Browser::new_page`](crate::Browser::new_page) 创建，
/// 使用完毕后调用 [`Page::close`] 关闭。
pub struct Page {
    /// CDP 会话（供 fetcher/stealth 等上层组合使用）。
    session: Arc<CdpSession>,
    session_id: String,
    pub(crate) frame_id: String,
    /// CDP target ID — 用于显式关闭 tab。
    /// `close()` 调用后置 `None`，防止 `Drop` 重复关闭。
    target_id: Option<String>,
}

impl Page {
    /// 获取 CDP 会话引用（供上层组合使用，不暴露字段本身）。
    pub fn session(&self) -> &Arc<CdpSession> {
        &self.session
    }

    /// Execute a raw CDP command on this page's target session.
    pub async fn cmd(&self, method: &str, params: Value) -> Result<Value> {
        self.session
            .execute_with_session(method, params, Some(&self.session_id))
            .await
    }

    /// 此 page 的 CDP session ID（每个 tab 独立）。
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 显式关闭此 tab（发送 `Target.closeTarget`）。
    ///
    /// 推荐在 async 上下文中调用以确保 tab 立即关闭。调用后 `target_id` 置
    /// `None`，`Drop` 时不会重复关闭。若忘记调用，`Drop` 会在后台 task 中
    /// 兜底关闭（但 runtime 关闭时 spawn 可能丢失）。
    pub async fn close(&mut self) -> Result<()> {
        if let Some(target_id) = self.target_id.take() {
            // Target.closeTarget 是 browser-level 命令，不带 session_id
            self.session
                .execute("Target.closeTarget", json!({ "targetId": target_id }))
                .await?;
        }
        Ok(())
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        // 兜底：若用户忘记调 `close()`，在后台 task 中关闭 tab。
        // 注意：Drop 是同步的无法 await，必须 spawn；
        // runtime 关闭时 spawn 可能丢失，但比完全不关闭强。
        if let Some(target_id) = self.target_id.take() {
            let session = Arc::clone(&self.session);
            tokio::spawn(async move {
                let _ = session
                    .execute("Target.closeTarget", json!({ "targetId": target_id }))
                    .await;
            });
        }
    }
}
