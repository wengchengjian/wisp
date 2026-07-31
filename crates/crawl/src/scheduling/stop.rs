//! 终止条件策略：Spider 的停止判定由可组合的策略对象实现。

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;

/// 终止上下文：派发请求前由引擎构造的只读快照。
#[derive(Debug, Clone)]
pub struct StopContext {
    /// 该 Spider 已爬页数
    pub pages: usize,
    /// 该 Spider 已产 item 数
    pub items: usize,
    /// 该 Spider 错误数
    pub errors: usize,
    /// 该 Spider 在飞请求数
    pub in_flight: usize,
    /// 该 Spider 已运行时长
    pub elapsed: Duration,
    /// 共享队列剩余请求数
    pub queue_size: usize,
    /// 各 callback 已爬页数（`"default"` 表示无 callback 的入口请求）
    pub callback_pages: HashMap<String, usize>,
}

impl StopContext {
    /// 指定 callback 已爬页数。
    pub fn page_count(&self, callback: &str) -> usize {
        self.callback_pages.get(callback).copied().unwrap_or(0)
    }
}

/// 终止策略 trait。返回 true 表示该 Spider 停止派发新请求。
pub trait StopCondition: Send + Sync {
    /// 检查是否应停止。
    fn should_stop(&self, ctx: &StopContext) -> bool;

    /// 是否依赖 `StopContext::callback_pages`。
    ///
    /// 默认 `false`，引擎可跳过快照分配；按 callback 计数或闭包策略需覆盖为 `true`。
    fn uses_callback_pages(&self) -> bool {
        false
    }

    /// 逻辑与组合。
    fn and<C: StopCondition + 'static>(self, other: C) -> Arc<dyn StopCondition>
    where
        Self: Sized + 'static,
    {
        Arc::new(And { a: Arc::new(self), b: Arc::new(other) })
    }
    /// 逻辑或组合。
    fn or<C: StopCondition + 'static>(self, other: C) -> Arc<dyn StopCondition>
    where
        Self: Sized + 'static,
    {
        Arc::new(Or { a: Arc::new(self), b: Arc::new(other) })
    }
    /// 逻辑非。
    fn not(self) -> Arc<dyn StopCondition>
    where
        Self: Sized + 'static,
    {
        Arc::new(Not { inner: Arc::new(self) })
    }
}

// === 原子策略 ===

/// 已爬页数达到上限。
pub struct MaxPages(pub usize);
impl StopCondition for MaxPages {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        ctx.pages >= self.0
    }
}

/// 已产 item 数达到上限。
pub struct MaxItems(pub usize);
impl StopCondition for MaxItems {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        ctx.items >= self.0
    }
}

/// 指定 callback 已爬页数达到上限。
///
/// 适合多级流程中的“业务阶段”计数，例如 `"detail"` 页代表一本书：
/// `pages_by_callback("detail", 20)` 即“爬取 20 本书后停止”。
pub struct MaxPagesByCallback {
    /// 要计数的 callback 名称。
    pub callback: String,
    /// 页面数量上限。
    pub limit: usize,
}

impl MaxPagesByCallback {
    /// 创建按 callback 计数的停止条件。
    pub fn new(callback: impl Into<String>, limit: usize) -> Self {
        Self {
            callback: callback.into(),
            limit,
        }
    }
}

impl StopCondition for MaxPagesByCallback {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        ctx.page_count(&self.callback) >= self.limit
    }

    fn uses_callback_pages(&self) -> bool {
        true
    }
}

/// 构造 [`MaxPagesByCallback`]：`pages_by_callback("detail", 20)`。
pub fn pages_by_callback(callback: &str, limit: usize) -> MaxPagesByCallback {
    MaxPagesByCallback::new(callback, limit)
}

/// 错误数达到上限。
pub struct MaxErrors(pub usize);
impl StopCondition for MaxErrors {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        ctx.errors >= self.0
    }
}

/// 运行时长达到上限。
pub struct Timeout(pub Duration);
impl StopCondition for Timeout {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        ctx.elapsed >= self.0
    }
}

/// 永不停止（默认）。
pub struct NeverStop;
impl StopCondition for NeverStop {
    fn should_stop(&self, _ctx: &StopContext) -> bool { false }
}

/// 闭包转 StopCondition。
pub struct FnStopCondition<F: Fn(&StopContext) -> bool + Send + Sync>(pub F);
impl<F: Fn(&StopContext) -> bool + Send + Sync> StopCondition for FnStopCondition<F> {
    fn should_stop(&self, ctx: &StopContext) -> bool { (self.0)(ctx) }

    fn uses_callback_pages(&self) -> bool {
        true
    }
}

// === 组合策略 ===

struct And { a: Arc<dyn StopCondition>, b: Arc<dyn StopCondition> }
impl StopCondition for And {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        self.a.should_stop(ctx) && self.b.should_stop(ctx)
    }

    fn uses_callback_pages(&self) -> bool {
        self.a.uses_callback_pages() || self.b.uses_callback_pages()
    }
}

struct Or { a: Arc<dyn StopCondition>, b: Arc<dyn StopCondition> }
impl StopCondition for Or {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        self.a.should_stop(ctx) || self.b.should_stop(ctx)
    }

    fn uses_callback_pages(&self) -> bool {
        self.a.uses_callback_pages() || self.b.uses_callback_pages()
    }
}

struct Not { inner: Arc<dyn StopCondition> }
impl StopCondition for Not {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        !self.inner.should_stop(ctx)
    }

    fn uses_callback_pages(&self) -> bool {
        self.inner.uses_callback_pages()
    }
}
