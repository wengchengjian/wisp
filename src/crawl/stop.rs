//! 终止条件策略：Spider 的停止判定由可组合的策略对象实现。

use std::sync::Arc;
use std::time::Duration;

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
}

/// 终止策略 trait。返回 true 表示该 Spider 停止派发新请求。
pub trait StopCondition: Send + Sync {
    fn should_stop(&self, ctx: &StopContext) -> bool;

    fn and<C: StopCondition + 'static>(self, other: C) -> Arc<dyn StopCondition>
    where
        Self: Sized + 'static,
    {
        Arc::new(And { a: Arc::new(self), b: Arc::new(other) })
    }
    fn or<C: StopCondition + 'static>(self, other: C) -> Arc<dyn StopCondition>
    where
        Self: Sized + 'static,
    {
        Arc::new(Or { a: Arc::new(self), b: Arc::new(other) })
    }
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
}

// === 组合策略 ===

struct And { a: Arc<dyn StopCondition>, b: Arc<dyn StopCondition> }
impl StopCondition for And {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        self.a.should_stop(ctx) && self.b.should_stop(ctx)
    }
}

struct Or { a: Arc<dyn StopCondition>, b: Arc<dyn StopCondition> }
impl StopCondition for Or {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        self.a.should_stop(ctx) || self.b.should_stop(ctx)
    }
}

struct Not { inner: Arc<dyn StopCondition> }
impl StopCondition for Not {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        !self.inner.should_stop(ctx)
    }
}
