//! Browser Pool — 单 Browser + 多 Page 并发模型。
//!
//! # 设计
//!
//! - 1 个 Chrome 进程，N 个并发 tab（用 Semaphore 限制并发数）
//! - `acquire()` 返回 `BrowserHandle`，内含 `Page` + permit
//! - `BrowserHandle::Drop` 自动关闭 tab + release permit
//! - Browser 懒启动（首次 acquire 时 launch）

mod core;
mod handle;

pub use core::BrowserPool;
pub use handle::BrowserHandle;

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_core::config::LaunchOptions;

    #[tokio::test]
    async fn test_pool_creation() {
        let pool = BrowserPool::new(4, LaunchOptions::default());
        assert!(!pool.is_launched().await);
        assert_eq!(pool.available_permits(), 4);
        assert_eq!(pool.max_concurrent_pages(), 4);
    }

    #[tokio::test]
    async fn test_permits_release_on_handle_drop() {
        // 不需要真实 Chrome：验证 permit 计数逻辑
        let pool = BrowserPool::new(2, LaunchOptions::default());
        assert_eq!(pool.available_permits(), 2);

        // 模拟：直接 acquire permit（不 launch browser）
        let permit1 = pool.page_permits.clone().acquire_owned().await.unwrap();
        let permit2 = pool.page_permits.clone().acquire_owned().await.unwrap();
        assert_eq!(pool.available_permits(), 0);

        // 释放一个 permit
        drop(permit1);
        assert_eq!(pool.available_permits(), 1);

        // 可以再 acquire
        let permit3 = pool.page_permits.clone().acquire_owned().await.unwrap();
        assert_eq!(pool.available_permits(), 0);

        // 清理
        drop(permit2);
        drop(permit3);
        assert_eq!(pool.available_permits(), 2);
    }

    // === ND-016-TEST：launch 失败恢复测试 ===
    //
    // 不需要真实 Chrome：用故意错误的 executable_path 触发 LaunchFailed。

    /// 构造指向不存在可执行文件的 LaunchOptions。
    fn broken_launch_options() -> LaunchOptions {
        LaunchOptions {
            headless: true,
            executable_path: Some(std::path::PathBuf::from("/nonexistent/wisp-broken-chrome")),
            ..Default::default()
        }
    }

    /// launch 失败（Chrome 不存在）时 acquire 应返回 Err。
    #[tokio::test]
    async fn launch_failure_returns_error() {
        let pool = BrowserPool::new(2, broken_launch_options());
        assert!(!pool.is_launched().await);
        assert_eq!(pool.available_permits(), 2);

        let result = pool.acquire().await;
        assert!(
            result.is_err(),
            "executable_path 不存在时 acquire 应返回 Err"
        );
        // 失败后 browser 仍为 None（未启动）
        assert!(!pool.is_launched().await);
        // 失败不影响 permit 计数（acquire 在 launch 前已获取 permit，但 launch 失败时 BrowserHandle::Drop 释放）
        assert_eq!(pool.available_permits(), 2, "失败后 permit 应全部归还");
    }

    /// launch 失败后下次 acquire 应重试（不永久卡住）。
    #[tokio::test]
    async fn launch_failure_then_retry() {
        let pool = BrowserPool::new(2, broken_launch_options());

        // 第一次失败
        let r1 = pool.acquire().await;
        assert!(r1.is_err());
        // 第二次仍失败（但应能再次尝试，不永久卡住）
        let r2 = pool.acquire().await;
        assert!(r2.is_err());
        // 第三次同样
        let r3 = pool.acquire().await;
        assert!(r3.is_err());

        // 3 次失败后 permit 仍全部归还
        assert_eq!(pool.available_permits(), 2);
        assert!(!pool.is_launched().await);
    }

    /// 并发 acquire：launch 失败时所有等待者都应收到错误，不死锁。
    #[tokio::test]
    async fn concurrent_acquire_all_fail_on_broken_launch() {
        let pool = BrowserPool::new(4, broken_launch_options());

        // 4 个并发 acquire，全部应失败
        let h1 = {
            let pool = pool.clone();
            tokio::spawn(async move { pool.acquire().await })
        };
        let h2 = {
            let pool = pool.clone();
            tokio::spawn(async move { pool.acquire().await })
        };
        let h3 = {
            let pool = pool.clone();
            tokio::spawn(async move { pool.acquire().await })
        };
        let h4 = {
            let pool = pool.clone();
            tokio::spawn(async move { pool.acquire().await })
        };

        let r1 = h1.await.unwrap();
        let r2 = h2.await.unwrap();
        let r3 = h3.await.unwrap();
        let r4 = h4.await.unwrap();

        assert!(r1.is_err() && r2.is_err() && r3.is_err() && r4.is_err());
        // 所有 permit 应归还（失败路径不持有 handle）
        assert_eq!(pool.available_permits(), 4);
    }

    /// shutdown 在未启动状态下应为 no-op（不 panic）。
    #[tokio::test]
    async fn shutdown_without_launch_is_noop() {
        let pool = BrowserPool::new(2, broken_launch_options());
        // 未启动直接 shutdown
        pool.shutdown().await;
        assert!(!pool.is_launched().await);
        // 可重复 shutdown
        pool.shutdown().await;
        assert!(!pool.is_launched().await);
    }

    /// shutdown 后 pool 不可再 acquire（browser 已 take()，但 launch_options 仍可用）。
    /// 注：当前实现 shutdown 后再 acquire 会重新 launch（broken options 仍失败）。
    #[tokio::test]
    async fn shutdown_then_acquire_relaunches() {
        let pool = BrowserPool::new(2, broken_launch_options());
        // 先失败一次
        assert!(pool.acquire().await.is_err());
        // shutdown（无 browser 可关，no-op）
        pool.shutdown().await;
        assert!(!pool.is_launched().await);
        // 再次 acquire 仍会尝试 launch（失败）
        assert!(pool.acquire().await.is_err());
    }
}
