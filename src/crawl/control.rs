//! Global crawl control — standalone async functions for pause/resume/cancel.
//!
//! Uses a process-wide static registry with a version counter (watch channel)
//! to wake up paused tasks. URL is the natural isolation key — no crawl name needed.
//!
//! # Usage
//! ```rust,no_run
//! use wisp::crawl::control;
//!
//! # async fn example() {
//! // From any task / API handler:
//! control::pause("https://example.com/slow").await;
//! control::resume("https://example.com/slow").await;
//! control::cancel("https://example.com/unwanted").await;
//! control::shutdown();
//! # }
//! ```

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::{watch, RwLock};

// === Global state (process-wide singleton) ===

static PAUSED_URLS: LazyLock<RwLock<HashSet<String>>> = LazyLock::new(|| RwLock::new(HashSet::new()));
static CANCELLED_URLS: LazyLock<RwLock<HashSet<String>>> = LazyLock::new(|| RwLock::new(HashSet::new()));
static GLOBAL_PAUSED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);

/// Version counter: bumped on every pause/resume/shutdown to wake waiting tasks.
static VERSION: LazyLock<watch::Sender<u64>> = LazyLock::new(|| watch::channel(0).0);

fn bump() {
    let v = *VERSION.borrow();
    VERSION.send(v.wrapping_add(1)).ok();
}

// === Public API: call from anywhere ===

/// Pause a specific URL. When the engine reaches this URL, it blocks until `resume()`.
pub async fn pause(url: &str) {
    PAUSED_URLS.write().await.insert(url.to_string());
    bump();
}

/// Resume a specific URL, waking any blocked task waiting on it.
pub async fn resume(url: &str) {
    PAUSED_URLS.write().await.remove(url);
    bump();
}

/// Cancel a specific URL. The engine will skip it silently.
pub async fn cancel(url: &str) {
    CANCELLED_URLS.write().await.insert(url.to_string());
}

/// Pause all crawling (all URLs block until `resume_all()`).
pub fn pause_all() {
    GLOBAL_PAUSED.store(true, Ordering::SeqCst);
    bump();
}

/// Resume all crawling.
pub fn resume_all() {
    GLOBAL_PAUSED.store(false, Ordering::SeqCst);
    bump();
}

/// Gracefully stop all crawling. In-flight requests finish, then the engine exits.
pub fn shutdown() {
    SHUTDOWN_FLAG.store(true, Ordering::SeqCst);
    bump();
}

/// Reset all control state (call before a new crawl run).
pub async fn reset() {
    PAUSED_URLS.write().await.clear();
    CANCELLED_URLS.write().await.clear();
    GLOBAL_PAUSED.store(false, Ordering::SeqCst);
    SHUTDOWN_FLAG.store(false, Ordering::SeqCst);
    bump();
}

// === Internal queries (called by Engine, pub(crate)) ===

pub(crate) async fn is_cancelled(url: &str) -> bool {
    CANCELLED_URLS.read().await.contains(url)
}

/// If the URL or global pause is active, block until resumed or shutdown.
/// Returns `false` if shutdown was detected (caller should terminate).
///
/// Wake mechanism: watches the VERSION channel; any pause/resume/shutdown
/// bumps the version, causing `rx.changed()` to return.
pub(crate) async fn wait_if_paused(url: &str) -> bool {
    let mut rx = VERSION.subscribe();
    loop {
        if SHUTDOWN_FLAG.load(Ordering::SeqCst) { return false; }
        let global = GLOBAL_PAUSED.load(Ordering::SeqCst);
        let url_paused = PAUSED_URLS.read().await.contains(url);
        if !global && !url_paused { return true; }
        // 阻塞直到 version 变化（resume/pause_all/shutdown 都会 bump）
        // 超时 60 秒作为极端 safety
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    // watch sender dropped（不应发生），退出避免死循环
                    return !SHUTDOWN_FLAG.load(Ordering::SeqCst);
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(60)) => {
                // safety fallback：60 秒后重新检查状态
            }
        }
    }
}

pub(crate) fn is_shutdown() -> bool {
    SHUTDOWN_FLAG.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    // All control tests in one function to avoid global state interference.
    #[tokio::test]
    async fn test_control_all() {
        reset().await;

        // pause / resume
        pause("https://example.com/a").await;
        assert!(PAUSED_URLS.read().await.contains("https://example.com/a"));
        resume("https://example.com/a").await;
        assert!(!PAUSED_URLS.read().await.contains("https://example.com/a"));

        // cancel
        cancel("https://example.com/bad").await;
        assert!(is_cancelled("https://example.com/bad").await);
        assert!(!is_cancelled("https://example.com/good").await);

        // shutdown flag
        assert!(!is_shutdown());
        shutdown();
        assert!(is_shutdown());
        reset().await;
        assert!(!is_shutdown());

        // wait_if_paused returns true when not paused
        assert!(wait_if_paused("https://example.com/free").await);

        // wait_if_paused returns false on shutdown
        shutdown();
        assert!(!wait_if_paused("https://example.com/any").await);
        reset().await;

        // pause + resume sequence
        pause("https://example.com/p1").await;
        assert!(PAUSED_URLS.read().await.contains("https://example.com/p1"));
        resume("https://example.com/p1").await;
        assert!(!PAUSED_URLS.read().await.contains("https://example.com/p1"));
        assert!(wait_if_paused("https://example.com/p1").await);
    }
}
