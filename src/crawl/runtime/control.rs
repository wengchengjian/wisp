//! 引擎级控制状态（per-Engine 隔离，解决 I4）。
//!
//! 原先为进程级全局 static（`PAUSED_URLS` / `SHUTDOWN_FLAG` 等），多 Engine 实例
//! 共享同一份状态，导致隔离性差。现重构为 `EngineControl` 结构体，由每个 `Engine`
//! 独立持有，多 Engine 实例控制状态完全隔离。
//!
//! # Usage
//! ```rust,no_run
//! use wisp::crawl::Engine;
//!
//! let engine = Engine::infra().build().unwrap();
//! let control = engine.control();
//! // 从任意 task / API handler 调用：
//! // control.pause("https://example.com/slow").await;
//! // control.shutdown();
//! ```

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, RwLock};

/// per-Engine 控制状态。
///
/// 持有与原全局 static 等价的字段，但每个 `Engine` 实例独立一份，
/// 多 Engine 之间状态完全隔离。
#[derive(Debug)]
pub struct EngineControl {
    paused_urls: Arc<RwLock<HashSet<String>>>,
    cancelled_urls: Arc<RwLock<HashSet<String>>>,
    global_paused: AtomicBool,
    shutdown: AtomicBool,
    /// 版本计数器：每次 pause/resume/shutdown 时 bump，唤醒等待中的 task。
    version: watch::Sender<u64>,
}

impl EngineControl {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(0u64);
        Self {
            paused_urls: Arc::new(RwLock::new(HashSet::new())),
            cancelled_urls: Arc::new(RwLock::new(HashSet::new())),
            global_paused: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            version: tx,
        }
    }

    fn bump(&self) {
        let _ = self.version.send(self.version.borrow().wrapping_add(1));
    }

    /// 暂停指定 URL。引擎处理到该 URL 时会阻塞直到 `resume()`。
    pub async fn pause(&self, url: &str) {
        self.paused_urls.write().await.insert(url.to_string());
        self.bump();
    }

    /// 恢复指定 URL，唤醒阻塞在该 URL 上的 task。
    pub async fn resume(&self, url: &str) {
        self.paused_urls.write().await.remove(url);
        self.bump();
    }

    /// 取消指定 URL。引擎会静默跳过。
    pub async fn cancel(&self, url: &str) {
        self.cancelled_urls.write().await.insert(url.to_string());
        self.bump();
    }

    /// 暂停所有爬取（所有 URL 阻塞直到 `resume_all()`）。
    pub fn pause_all(&self) {
        self.global_paused.store(true, Ordering::SeqCst);
        self.bump();
    }

    /// 恢复所有爬取。
    pub fn resume_all(&self) {
        self.global_paused.store(false, Ordering::SeqCst);
        self.bump();
    }

    /// 优雅关闭：in-flight 请求完成后引擎退出。
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.bump();
    }

    /// 重置所有控制状态（每次 run 开始前调用）。
    pub async fn reset(&self) {
        self.paused_urls.write().await.clear();
        self.cancelled_urls.write().await.clear();
        self.global_paused.store(false, Ordering::SeqCst);
        self.shutdown.store(false, Ordering::SeqCst);
        self.bump();
    }

    pub async fn is_cancelled(&self, url: &str) -> bool {
        self.cancelled_urls.read().await.contains(url)
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// 若该 URL 或全局暂停生效，阻塞直到 resume 或 shutdown。
    ///
    /// 返回 `false` 表示检测到 shutdown（调用方应终止）。
    ///
    /// 唤醒机制：监听 `version` watch channel；任何 pause/resume/shutdown
    /// 都会 bump 版本，使 `rx.changed()` 返回。
    pub async fn wait_if_paused(&self, url: &str) -> bool {
        let mut rx = self.version.subscribe();
        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                return false;
            }
            let global = self.global_paused.load(Ordering::SeqCst);
            let url_paused = self.paused_urls.read().await.contains(url);
            if !global && !url_paused {
                return true;
            }
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        // watch sender dropped（不应发生），退出避免死循环
                        return !self.shutdown.load(Ordering::SeqCst);
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(60)) => {
                    // safety fallback：60 秒后重新检查状态
                }
            }
        }
    }
}

impl Default for EngineControl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_engine_control_pause_resume() {
        let ctrl = EngineControl::new();
        ctrl.pause("https://example.com/a").await;
        assert!(ctrl.paused_urls.read().await.contains("https://example.com/a"));
        ctrl.resume("https://example.com/a").await;
        assert!(!ctrl.paused_urls.read().await.contains("https://example.com/a"));
    }

    #[tokio::test]
    async fn test_engine_control_cancel() {
        let ctrl = EngineControl::new();
        ctrl.cancel("https://example.com/bad").await;
        assert!(ctrl.is_cancelled("https://example.com/bad").await);
        assert!(!ctrl.is_cancelled("https://example.com/good").await);
    }

    #[tokio::test]
    async fn test_engine_control_shutdown_and_reset() {
        let ctrl = EngineControl::new();
        assert!(!ctrl.is_shutdown());
        ctrl.shutdown();
        assert!(ctrl.is_shutdown());
        ctrl.reset().await;
        assert!(!ctrl.is_shutdown());
    }

    #[tokio::test]
    async fn test_engine_control_wait_if_paused_free() {
        let ctrl = EngineControl::new();
        assert!(ctrl.wait_if_paused("https://example.com/free").await);
    }

    #[tokio::test]
    async fn test_engine_control_wait_if_paused_shutdown() {
        let ctrl = EngineControl::new();
        ctrl.shutdown();
        assert!(!ctrl.wait_if_paused("https://example.com/any").await);
    }

    #[tokio::test]
    async fn test_engine_control_isolation_between_instances() {
        // 验证两个 EngineControl 实例状态完全隔离
        let ctrl_a = EngineControl::new();
        let ctrl_b = EngineControl::new();

        ctrl_a.pause_all();
        ctrl_a.shutdown();

        // B 不受 A 影响
        assert!(!ctrl_b.is_shutdown());
        assert!(ctrl_b.wait_if_paused("https://example.com/x").await);
    }
}
