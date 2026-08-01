//! Engine 子模块：guard。

use super::*;

// === InFlightGuard ===

/// In-flight 计数守卫：drop 时递减计数并通知主循环。
///
/// 关键修复：drop 时必须通知 `work_notify`，否则当请求失败（process_request 返回 None，
/// process_response 不被调用）时，主循环在 scheduler 空且 in-flight > 0 的状态下
/// 会永远等待 work_notify，导致死锁。
pub(crate) struct InFlightGuard {
    pub counter: Arc<AtomicUsize>,
    /// 全局计数器 drop 时需要唤醒主循环；per-spider 计数无需重复唤醒。
    pub work_notify: Option<Arc<tokio::sync::Notify>>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
        // 唤醒主循环：in-flight 计数变化，主循环需要重新检查退出条件
        if let Some(notify) = &self.work_notify {
            notify.notify_one();
        }
    }
}
