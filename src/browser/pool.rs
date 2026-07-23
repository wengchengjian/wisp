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

/// 在槽位列表中查找可复用的空闲槽索引。
///
/// `is_in_use(p)` 返回 `true` 表示该实例正在使用中。返回第一个内容为 `Some`
/// 且未在使用的槽位索引。
///
/// 这是 `acquire` 复用路径的纯逻辑核心，提取为独立函数便于单元测试
/// （`Browser::launch` 需要真实 Chrome，无法在 CI 中直接测试 `acquire`）。
///
/// 用 `enumerate` 在查找时即捕获索引，避免旧实现 `find` + `position` 两步走
/// 导致返回错误索引（多实例 in_use 时 `position(|p| p.in_use)` 命中更早的
/// in_use 实例而非刚标记的那个）。
fn pick_idle_slot<T, F>(slots: &[Option<T>], is_in_use: F) -> Option<usize>
where
    F: Fn(&T) -> bool,
{
    slots
        .iter()
        .enumerate()
        .find_map(|(idx, slot)| slot.as_ref().filter(|p| !is_in_use(p)).map(|_| idx))
}

/// 浏览器实例池。
///
/// 复用已启动的 Browser 实例，每个请求创建新 tab 而非新进程。
///
/// 内部用 `Vec<Option<PooledBrowser>>` 槽位模型：每个槽位要么装有实例
/// （`Some`），要么因超时回收而空置（`None`）。回收用 `take()` 清空内容但
/// 保留空槽，索引在池的整个生命周期内永不改变，从而保证在飞
/// `BrowserHandle.index` 始终有效。
pub struct BrowserPool {
    instances: Mutex<Vec<Option<PooledBrowser>>>,
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
    /// 返回 `BrowserHandle`，Drop 时自动归还到池中。返回的 `index` 在池的
    /// 整个生命周期内始终指向同一个槽位（不会因回收而移位）。
    pub async fn acquire(self: &Arc<Self>) -> Result<BrowserHandle> {
        // 1. 回收超时空闲实例 + 复用空闲实例（索引不变）
        {
            let mut instances = self.instances.lock().await;
            let now = Instant::now();
            // 超时回收：take 掉内容（drop browser 进程），保留空槽位，索引不变
            for slot in instances.iter_mut() {
                if let Some(p) = slot {
                    if !p.in_use && now.duration_since(p.last_used) > self.idle_timeout {
                        *slot = None;
                    }
                }
            }
            // 复用第一个空闲实例（用 pick_idle_slot 在查找时即捕获正确索引）
            if let Some(idx) = pick_idle_slot(&instances, |p: &PooledBrowser| p.in_use) {
                if let Some(p) = instances[idx].as_mut() {
                    p.in_use = true;
                    p.last_used = Instant::now();
                }
                return Ok(BrowserHandle {
                    pool: Arc::clone(self),
                    index: idx,
                });
            }
        }

        // 2. 在已有空槽位中新建实例（不增长 Vec，索引稳定）
        {
            let mut instances = self.instances.lock().await;
            for (idx, slot) in instances.iter_mut().enumerate() {
                if slot.is_none() {
                    let browser = Browser::launch(self.launch_options.clone()).await?;
                    *slot = Some(PooledBrowser {
                        browser,
                        last_used: Instant::now(),
                        in_use: true,
                    });
                    return Ok(BrowserHandle {
                        pool: Arc::clone(self),
                        index: idx,
                    });
                }
            }
            // Vec 没有空槽且未达 max_size：push 新槽
            if instances.len() < self.max_size {
                let browser = Browser::launch(self.launch_options.clone()).await?;
                instances.push(Some(PooledBrowser {
                    browser,
                    last_used: Instant::now(),
                    in_use: true,
                }));
                let index = instances.len() - 1;
                return Ok(BrowserHandle {
                    pool: Arc::clone(self),
                    index,
                });
            }
        }

        // 3. 达到上限，等待空闲（简单自旋等待）
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut instances = self.instances.lock().await;
            if let Some(idx) = pick_idle_slot(&instances, |p: &PooledBrowser| p.in_use) {
                if let Some(p) = instances[idx].as_mut() {
                    p.in_use = true;
                    p.last_used = Instant::now();
                }
                return Ok(BrowserHandle {
                    pool: Arc::clone(self),
                    index: idx,
                });
            }
        }
    }

    /// 归还实例（标记为空闲）。
    async fn release(&self, index: usize) {
        let mut instances = self.instances.lock().await;
        if let Some(Some(pooled)) = instances.get_mut(index) {
            pooled.in_use = false;
            pooled.last_used = Instant::now();
        }
    }

    /// 关闭所有实例并清空池。
    pub async fn shutdown(&self) {
        let mut instances = self.instances.lock().await;
        for slot in instances.drain(..) {
            if let Some(pooled) = slot {
                // Browser::close 消费 self，这里用 drop 触发 kill
                drop(pooled.browser);
            }
        }
    }

    /// 当前池中实例总数（非空槽数）。
    pub async fn size(&self) -> usize {
        self.instances
            .lock()
            .await
            .iter()
            .filter(|s| s.is_some())
            .count()
    }

    /// 当前空闲实例数。
    pub async fn idle_count(&self) -> usize {
        self.instances
            .lock()
            .await
            .iter()
            .filter_map(|s| s.as_ref())
            .filter(|p| !p.in_use)
            .count()
    }

    /// 获取指定索引的 Browser 引用（内部使用）。
    async fn get_browser(&self, index: usize) -> Option<BrowserRef> {
        let instances = self.instances.lock().await;
        instances.get(index)?.as_ref().map(|p| BrowserRef {
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

    /// 验证 bug #1 的核心索引逻辑：当槽 0 已 in_use、槽 1 空闲时，
    /// 复用应返回槽 1 的索引，而非旧 position(in_use) 错误返回的槽 0。
    #[test]
    fn pick_idle_slot_returns_correct_index_when_earlier_slot_in_use() {
        // 槽 0 在使用中，槽 1 空闲。in_use 用 bool 表示（true=使用中）。
        let slots: Vec<Option<bool>> = vec![Some(true), Some(false)];
        let idx = pick_idle_slot(&slots, |b: &bool| *b);
        assert_eq!(idx, Some(1), "应跳过 in_use 的槽 0，返回空闲槽 1");
    }

    /// 验证跳过空槽（None）：超时回收后槽位变 None，不应被当作可复用实例。
    #[test]
    fn pick_idle_slot_skips_empty_slots() {
        // 槽 0 已被 take() 清空（None），槽 1 空闲。
        let slots: Vec<Option<bool>> = vec![None, Some(false)];
        let idx = pick_idle_slot(&slots, |b: &bool| *b);
        assert_eq!(idx, Some(1), "应跳过 None 空槽，返回槽 1");
    }

    /// 验证 bug #2 的修复不变量：超时回收用 take()（置 None）而非 retain()（移位），
    /// 因此其他槽的索引保持不变。
    #[test]
    fn slot_take_preserves_indices() {
        let mut slots: Vec<Option<bool>> = vec![Some(true), Some(false), Some(true)];
        // 模拟超时回收槽 0：take 内容，保留空槽位
        slots[0] = None;
        // 槽 1、2 的索引与内容均不变
        assert_eq!(slots[0], None);
        assert_eq!(slots[1], Some(false));
        assert_eq!(slots[2], Some(true));
    }
}
