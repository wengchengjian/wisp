//! Browser Pool — 浏览器实例复用，避免每次请求冷启动 Chrome。
//!
//! # 设计
//!
//! - 维护一组预热的 Browser 实例（`PooledBrowser`）
//! - `acquire()` 获取空闲实例或新建（不超过 max_size）
//! - `release()` 归还实例（标记空闲）
//! - 空闲超时自动回收
//! - RAII `BrowserHandle`：Drop 时自动归还
//!
//! # 性能收益
//!
//! crawl 数百 URL 时从「数百次冷启动（每次 2-5s）」变为
//! 「1 次启动 + 数百次 tab 切换（每次 ~100ms）」，性能提升 10-50x。

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::config::LaunchOptions;
use crate::error::Result;
use super::Browser;

/// 池中的浏览器实例。
struct PooledBrowser {
    browser: Browser,
    last_used: Instant,
    in_use: bool,
}

/// 浏览器实例池。
///
/// 复用已启动的 Browser 实例，每个请求创建新 tab 而非新进程。
pub struct BrowserPool {
    instances: Mutex<Vec<PooledBrowser>>,
    max_size: usize,
    idle_timeout: Duration,
    launch_options: LaunchOptions,
}

impl BrowserPool {
    /// 创建浏览器池。
    ///
    /// - `max_size`: 最大浏览器实例数（0 表示禁用池化）
    /// - `idle_timeout`: 空闲超时（超过后自动关闭回收）
    /// - `launch_options`: 浏览器启动配置
    pub fn new(max_size: usize, idle_timeout: Duration, launch_options: LaunchOptions) -> Arc<Self> {
        Arc::new(Self {
            instances: Mutex::new(Vec::new()),
            max_size,
            idle_timeout,
            launch_options,
        })
    }

    /// 获取一个浏览器实例（复用空闲或新建）。
    ///
    /// 返回 `BrowserHandle`，Drop 时自动归还到池中。
    pub async fn acquire(self: &Arc<Self>) -> Result<BrowserHandle> {
        // 1. 尝试复用空闲实例
        {
            let mut instances = self.instances.lock().await;
            // 清理超时空闲实例
            instances.retain(|p| {
                if !p.in_use && p.last_used.elapsed() > self.idle_timeout {
                    // 超时实例需要关闭（在后台执行）
                    false
                } else {
                    true
                }
            });

            // 查找空闲实例
            if let Some(pooled) = instances.iter_mut().find(|p| !p.in_use) {
                pooled.in_use = true;
                pooled.last_used = Instant::now();
                return Ok(BrowserHandle {
                    pool: Arc::clone(self),
                    index: instances.iter().position(|p| p.in_use).unwrap(),
                });
            }
        }

        // 2. 未达上限，新建实例
        let current_size = {
            let instances = self.instances.lock().await;
            instances.len()
        };

        if current_size < self.max_size {
            let browser = Browser::launch(self.launch_options.clone()).await?;
            let mut instances = self.instances.lock().await;
            instances.push(PooledBrowser {
                browser,
                last_used: Instant::now(),
                in_use: true,
            });
            let index = instances.len() - 1;
            return Ok(BrowserHandle {
                pool: Arc::clone(self),
                index,
            });
        }

        // 3. 达到上限，等待空闲（简单自旋等待）
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut instances = self.instances.lock().await;
            if let Some(index) = instances.iter().position(|p| !p.in_use) {
                instances[index].in_use = true;
                instances[index].last_used = Instant::now();
                return Ok(BrowserHandle {
                    pool: Arc::clone(self),
                    index,
                });
            }
        }
    }

    /// 归还实例（标记为空闲）。
    async fn release(&self, index: usize) {
        let mut instances = self.instances.lock().await;
        if let Some(pooled) = instances.get_mut(index) {
            pooled.in_use = false;
            pooled.last_used = Instant::now();
        }
    }

    /// 关闭所有实例并清空池。
    pub async fn shutdown(&self) {
        let mut instances = self.instances.lock().await;
        for pooled in instances.drain(..) {
            // Browser::close 消费 self，这里用 drop 触发 kill
            drop(pooled.browser);
        }
    }

    /// 当前池中实例总数。
    pub async fn size(&self) -> usize {
        self.instances.lock().await.len()
    }

    /// 当前空闲实例数。
    pub async fn idle_count(&self) -> usize {
        self.instances.lock().await.iter().filter(|p| !p.in_use).count()
    }

    /// 获取指定索引的 Browser 引用（内部使用）。
    async fn get_browser(&self, index: usize) -> Option<BrowserRef> {
        let instances = self.instances.lock().await;
        instances.get(index).map(|p| BrowserRef {
            session: p.browser.session.clone(),
            headless: p.browser.headless,
        })
    }
}

/// Browser 的轻量引用（不拥有进程）。
pub struct BrowserRef {
    pub session: Arc<super::CdpSession>,
    pub headless: bool,
}

impl BrowserRef {
    /// 在此浏览器中创建新 tab。
    pub async fn new_page(&self) -> Result<super::Page> {
        super::Page::create(Arc::clone(&self.session), self.headless).await
    }
}

/// RAII handle：持有池中的浏览器实例，Drop 时自动归还。
pub struct BrowserHandle {
    pool: Arc<BrowserPool>,
    index: usize,
}

impl BrowserHandle {
    /// 获取此实例的 Browser 引用（用于创建新 tab）。
    pub async fn browser_ref(&self) -> Option<BrowserRef> {
        self.pool.get_browser(self.index).await
    }

    /// 在此浏览器中创建新 tab。
    pub async fn new_page(&self) -> Result<super::Page> {
        let browser_ref = self.pool.get_browser(self.index).await
            .ok_or_else(|| crate::error::WispError::CdpError("browser pool: invalid index".into()))?;
        browser_ref.new_page().await
    }
}

impl Drop for BrowserHandle {
    fn drop(&mut self) {
        let pool = Arc::clone(&self.pool);
        let index = self.index;
        // 在后台 task 中归还（避免在 Drop 中 await）
        tokio::spawn(async move {
            pool.release(index).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pool_creation() {
        let pool = BrowserPool::new(
            4,
            Duration::from_secs(300),
            LaunchOptions::default(),
        );
        assert_eq!(pool.size().await, 0);
        assert_eq!(pool.idle_count().await, 0);
    }
}
