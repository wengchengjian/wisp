//! SpiderBuilder 与 handler 注册逻辑。

mod handlers;
mod sitemap;

use crate::builder::closure::BlockedFn;
use crate::stop::StopCondition;
use crate::{NeverStop, Request, Response};
use futures::future::BoxFuture;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

/// 异步 handler 签名：接收 Response，返回 (items, follows)。
///
/// 用 `Arc<dyn Fn(...) -> BoxFuture>` 让闭包可 Clone + 异步 + Send + Sync。
/// 每个 handler 捕获不同状态都满足同一签名。
pub type Handler =
    Arc<dyn Fn(Response) -> BoxFuture<'static, (Vec<Value>, Vec<Request>)> + Send + Sync>;

/// 闭包式 Spider 构建器。
///
/// 允许通过链式调用 + 闭包定义 Spider，避免为简单爬虫手写 trait impl。
///
/// ND-031-ARCH 修复：引擎配置（fetch_mode/obey_robots/max_retries/download_delay/auto_rules）
/// 已从 SpiderBuilder 移除，改用 `EngineBuilder` 配置。SpiderBuilder 只保留业务逻辑配置。
pub struct SpiderBuilder {
    name: String,
    start_urls: Vec<String>,
    handlers: HashMap<String, Handler>,
    allowed_domains: HashSet<String>,
    is_blocked_fn: Option<BlockedFn>,
    until_cond: Arc<dyn StopCondition>,
    max_depth: u32,
}

impl SpiderBuilder {
    /// 创建新 SpiderBuilder（name 为必填）。
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            start_urls: Vec::new(),
            handlers: HashMap::new(),
            allowed_domains: HashSet::new(),
            is_blocked_fn: None,
            until_cond: Arc::new(NeverStop),
            max_depth: u32::MAX,
        }
    }

    /// 设置起始 URL 列表。
    pub fn start_urls(mut self, urls: Vec<impl Into<String>>) -> Self {
        self.start_urls = urls.into_iter().map(|u| u.into()).collect();
        self
    }

    /// 设置允许的域名集合。
    pub fn allowed_domains(mut self, domains: Vec<impl Into<String>>) -> Self {
        self.allowed_domains = domains.into_iter().map(|d| d.into()).collect();
        self
    }

    /// 设置最大爬取深度（由 Engine 统一执行）。
    pub fn max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// 自定义阻塞检测逻辑。
    pub fn is_blocked<F>(mut self, f: F) -> Self
    where
        F: Fn(&Response) -> bool + Send + Sync + 'static,
    {
        self.is_blocked_fn = Some(Box::new(f));
        self
    }

    /// 注册 handler。label 为 `"default"` 表示入口（无 callback 时调用）。
    ///
    /// 多 callback 路由：`resp.follow_with(url, "detail")` 产生的请求会被
    /// `on("detail", handler)` 注册的 handler 处理。
    ///
    /// 这是定义 Spider 解析逻辑的唯一 API：至少注册一个 handler（通常为
    /// `"default"`）才能 `build()`。
    pub fn on<F, Fut>(mut self, label: &str, handler: F) -> Self
    where
        F: Fn(Response) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = (Vec<Value>, Vec<Request>)> + Send + 'static,
    {
        let boxed: Handler = Arc::new(move |resp| Box::pin(handler(resp)));
        self.handlers.insert(label.to_string(), boxed);
        self
    }

    /// 设置终止条件策略。
    pub fn until<C: StopCondition + 'static>(mut self, cond: C) -> Self {
        self.until_cond = Arc::new(cond);
        self
    }

    /// 构建 ClosureSpider 实例。
    ///
    /// # Panics
    /// 若未注册任何 handler（`on()` 未调用）则 panic。
    pub fn build(self) -> crate::builder::closure::ClosureSpider {
        assert!(
            !self.handlers.is_empty(),
            "SpiderBuilder: 必须至少注册一个 handler（通过 on()）"
        );

        crate::builder::closure::ClosureSpider {
            name: self.name,
            start_urls: self.start_urls,
            handlers: self.handlers,
            allowed_domains: self.allowed_domains,
            is_blocked_fn: self.is_blocked_fn,
            until_cond: self.until_cond,
            max_depth: self.max_depth,
        }
    }
}
