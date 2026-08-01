//! Browser Pool 实现。

use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore};

use super::handle::BrowserHandle;
use crate::Browser;
use wisp_core::config::LaunchOptions;
use wisp_core::error::{BrowserError, Result, WispError};
/// 浏览器池：单 Browser + 多 Page 并发。
///
/// 持有 1 个懒启动的 Browser 实例，通过 Semaphore 限制并发 page 数。
/// `acquire()` 返回 `BrowserHandle`（内含 `Page` + permit），Drop 自动清理。
pub struct BrowserPool {
    /// 懒启动的单个 Browser 实例。
    /// 用 `Mutex<Option<Arc<Browser>>>` 而非 `OnceCell`，以便 `shutdown` 能 `take()` 取出关闭。
    browser: Mutex<Option<Arc<Browser>>>,
    /// 限制并发 page 数。
    pub(crate) page_permits: Arc<Semaphore>,
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
            .map_err(|_| {
                WispError::Browser(BrowserError::Other("page_permits semaphore closed".into()))
            })?;

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
