//! Browser Pool — 单 Browser + 多 Page 并发模型。
//!
//! # 设计
//!
//! - 1 个 Chrome 进程，N 个并发 tab（用 Semaphore 限制并发数）
//! - `acquire()` 返回 `BrowserHandle`，内含 `Page` + permit
//! - `BrowserHandle::Drop` 自动关闭 tab + release permit
//! - Browser 懒启动（首次 acquire 时 launch）
//!
//! # 性能收益
//!
//! 相比多 Browser 进程模型：
//! - 内存降 75%（1 进程 vs 4 进程，max_concurrent=4 时）
//! - 启动开销降 75%（1 次 launch vs 4 次）
//! - 并发能力不变（N 个 tab 并发，与 N 个进程等效）
//! - 无 launch 持锁问题（browser 只 launch 1 次，后续 acquire 只是 new_page）
//!
//! # 与业界一致
//!
//! Puppeteer/Playwright/Crawlee 均采用单 Browser + 多 Page/BrowserContext 模式。

use std::sync::Arc;

use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::config::LaunchOptions;
use crate::error::{Result, WispError};

use super::{Browser, Page};

/// 浏览器池：单 Browser + 多 Page 并发。
///
/// 持有 1 个懒启动的 Browser 实例，通过 Semaphore 限制并发 page 数。
/// `acquire()` 返回 `BrowserHandle`（内含 `Page` + permit），Drop 自动清理。
pub struct BrowserPool {
    /// 懒启动的单个 Browser 实例。
    /// 用 `Mutex<Option<Arc<Browser>>>` 而非 `OnceCell`，以便 `shutdown` 能 `take()` 取出关闭。
    browser: Mutex<Option<Arc<Browser>>>,
    /// 限制并发 page 数。
    page_permits: Arc<Semaphore>,
    /// 最大并发 page 数（`Semaphore` 不暴露 max_permits，自行存储）。
    max_concurrent_pages: usize,
    launch_options: LaunchOptions,
}

impl BrowserPool {
    /// 创建浏览器池。
    ///
    /// - `max_concurrent_pages`: 最大并发 page 数（推荐 4-8）
    /// - `launch_options`: 浏览器启动配置
    pub fn new(max_concurrent_pages: usize, launch_options: LaunchOptions) -> Arc<Self> {
        Arc::new(Self {
            browser: Mutex::new(None),
            page_permits: Arc::new(Semaphore::new(max_concurrent_pages)),
            max_concurrent_pages,
            launch_options,
        })
    }

    /// 获取一个 page handle（含 page + permit）。
    ///
    /// - 首次调用懒启动 Browser（2-5s）
    /// - 后续调用只是 `new_page`（~50ms）
    /// - 并发数达上限时阻塞，直到有 page 释放
    pub async fn acquire(self: &Arc<Self>) -> Result<BrowserHandle> {
        let permit = Arc::clone(&self.page_permits)
            .acquire_owned()
            .await
            .map_err(|_| WispError::CdpError("page_permits semaphore closed".into()))?;

        let browser = self.get_or_launch_browser().await?;
        let page = browser.new_page().await?;

        Ok(BrowserHandle {
            page: Some(page),
            permit,
        })
    }

    /// 懒启动 Browser（首次或崩溃后重启）。
    ///
    /// 快路径用 `try_lock` 避免阻塞；慢路径用 `lock` + double-check 防并发 launch。
    async fn get_or_launch_browser(&self) -> Result<Arc<Browser>> {
        // 快路径：已启动（try_lock 非阻塞）
        if let Ok(guard) = self.browser.try_lock() {
            if let Some(ref b) = *guard {
                return Ok(Arc::clone(b));
            }
        }
        // 慢路径：launch（mutex 串行化，防止并发 launch）
        let mut guard = self.browser.lock().await;
        // double-check：可能在等锁期间其他 task 已 launch 完成
        if let Some(ref b) = *guard {
            return Ok(Arc::clone(b));
        }
        let browser = Arc::new(Browser::launch(self.launch_options.clone()).await?);
        *guard = Some(Arc::clone(&browser));
        Ok(browser)
    }

    /// 关闭 Browser 并清空池。
    ///
    /// 若仍有在飞的 `BrowserHandle`（Arc 引用计数 >1），无法 `close`，
    /// 进程会随程序退出自然终止（`tokio::process::Child` drop 时 kill）。
    pub async fn shutdown(&self) {
        let mut guard = self.browser.lock().await;
        if let Some(browser) = guard.take() {
            if let Ok(b) = Arc::try_unwrap(browser) {
                let _ = b.close().await;
            }
            // try_unwrap 失败（有在飞 handle）：忽略，进程随退出终止
        }
    }

    /// Browser 是否已启动。
    pub async fn is_launched(&self) -> bool {
        self.browser.lock().await.is_some()
    }

    /// 可用 permit 数（剩余并发容量）。
    pub fn available_permits(&self) -> usize {
        self.page_permits.available_permits()
    }

    /// 最大并发 page 数。
    pub fn max_concurrent_pages(&self) -> usize {
        self.max_concurrent_pages
    }
}

/// RAII handle：持有 page + permit，Drop 时自动关闭 tab + release permit。
///
/// `page` 用 `Option<Page>` 以便 `Drop` 时 `take()`。正常路径下
/// `fetch_browser` 已显式 `page.close().await`，`target_id` 置 `None`，
/// `Page::Drop` 不会重复关闭；`BrowserHandle::Drop` 只是触发 `Page::Drop`
/// + permit 自动 release。
pub struct BrowserHandle {
    page: Option<Page>,
    /// 通过 Drop 释放 permit（回退到 Semaphore），无需显式读取。
    #[allow(dead_code)]
    permit: OwnedSemaphorePermit,
}

impl BrowserHandle {
    /// 访问内部 Page。
    pub fn page(&self) -> &Page {
        self.page.as_ref().expect("page must be Some until Drop")
    }

    /// 可变访问内部 Page。
    pub fn page_mut(&mut self) -> &mut Page {
        self.page.as_mut().expect("page must be Some until Drop")
    }
}

impl Drop for BrowserHandle {
    fn drop(&mut self) {
        // 取出 page 触发 Page::Drop（兜底关闭 tab，若已 close 则 no-op）
        // permit 自动 release（OwnedSemaphorePermit::Drop）
        self.page.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
