//! Builtin 中间件子模块：retry。

use async_trait::async_trait;
use std::time::Duration;

use super::{CrawlContext, Middleware, ResponseMwAction};
use crate::Response;
use wisp_fetcher::FetchMode;

// === 重试类 ===

/// 阻塞重试中间件：检测 403/429/503 等阻塞状态码，通过 Refetch 自动重试。
///
/// 职责单一（修复 ND-032-CORR：原 `_retry` 计数与 `RetryMiddleware` 共享 meta 字段冲突）：
/// - **只决定**：响应是否被阻塞、是否值得重试
/// - **不维护**：Refetch 计数由 engine 在 `process_response` 内通过 `refetch_depth` 管理，
///   上限 `EngineConfig.max_refetch_rounds`
///
/// 原 `meta["_retry"]` 计数已移除，避免与 `RetryMiddleware` 的网络错误重试计数冲突。
/// 现在两套重试完全独立：
/// - 网络错误重试：`req.retry_count`（engine 维护，上限 `max_retries`）
/// - 阻塞重试：`refetch_depth`（engine 维护，上限 `max_refetch_rounds`）
pub struct BlockedRetryMiddleware {
    retry_delay: Duration,
}

impl BlockedRetryMiddleware {
    /// 创建阻塞重试中间件。
    pub fn new(retry_delay: Duration) -> Self {
        Self { retry_delay }
    }
}

impl Default for BlockedRetryMiddleware {
    fn default() -> Self {
        Self {
            retry_delay: Duration::from_millis(500),
        }
    }
}

#[async_trait]
impl Middleware for BlockedRetryMiddleware {
    fn priority(&self) -> u32 {
        80
    }

    async fn process_response(&self, resp: &mut Response, _ctx: &CrawlContext) -> ResponseMwAction {
        use crate::BLOCKED_STATUS_CODES;
        // Stealth 模式下不再 Refetch：浏览器已是最强抓取模式，重复 Refetch 只会
        // 再次触发 CF 挑战流程，无法突破拦截。状态码 403/503 通常是挑战前的响应，
        // 挑战已由 ChallengeSolver 解决（fetch_browser 内会修正最终状态码）。
        // 与 StealthUpgradeMiddleware 的防御保持一致。
        if resp.request.fetch_mode_override == Some(FetchMode::Stealth) {
            return ResponseMwAction::Continue;
        }
        if BLOCKED_STATUS_CODES.contains(&resp.status) {
            // 不再维护 _retry 计数，依赖 engine 的 refetch_depth 上限
            if !self.retry_delay.is_zero() {
                tokio::time::sleep(self.retry_delay).await;
            }
            let new_req = resp.request.clone();
            return ResponseMwAction::Refetch(new_req);
        }
        ResponseMwAction::Continue
    }
}
