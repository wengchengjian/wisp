//! 中间件链架构 — Scrapy 风格的可组合请求/响应拦截器。
//!
//! # 设计
//!
//! - `Middleware` trait：请求发出前/响应返回后的拦截点（等价 Scrapy Downloader Middleware）
//! - `ItemPipeline` trait：Item 顺序处理管道（等价 Scrapy Item Pipeline）
//! - 内建中间件：UA 轮换、代理注入、重试、Cookie、Robots
//!
//! # 示例
//!
//! ```rust,no_run
//! use wisp::crawl::middleware::{UaRotationMiddleware, RetryMiddleware};
//! use wisp::crawl::SpiderBuilder;
//!
//! let spider = SpiderBuilder::new("example")
//!     .start_urls(vec!["https://example.com/"])
//!     .middleware(UaRotationMiddleware::desktop())
//!     .middleware(RetryMiddleware::new(3, std::time::Duration::from_secs(1)))
//!     .on("default", |resp| async move { (vec![], vec![]) })
//!     .build();
//! ```

pub mod builtin;
pub mod pipeline;

pub use builtin::*;
pub use pipeline::*;

use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;

use crate::crawl::{SpiderRequest, SpiderResponse};
use crate::fetcher::FetchMode;

// === 中间件动作 ===

/// 中间件处理结果。
#[derive(Debug, Clone)]
pub enum MwAction {
    /// 继续传递给下一个中间件
    Continue,
    /// 已修改请求/响应，继续传递
    Modified,
    /// 跳过此请求（不再继续传递，不发送）
    Skip,
    /// 终止整个爬取，附带原因
    Abort(String),
    /// 用修改后的请求重新获取（用于 Cookie 挑战、JS 重定向等需要"检测→修改→重发"的场景）
    Refetch(SpiderRequest),
    /// 短路：直接返回响应，跳过实际网络请求（用于缓存命中）
    Respond(SpiderResponse),
}

impl PartialEq for MwAction {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Continue, Self::Continue) => true,
            (Self::Modified, Self::Modified) => true,
            (Self::Skip, Self::Skip) => true,
            (Self::Abort(a), Self::Abort(b)) => a == b,
            (Self::Refetch(a), Self::Refetch(b)) => a.url == b.url,
            (Self::Respond(a), Self::Respond(b)) => a.url == b.url,
            _ => false,
        }
    }
}

/// 错误处理动作。
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorAction {
    /// 继续传播错误（默认）
    Propagate,
    /// 重试此请求
    Retry,
    /// 忽略错误，跳过此请求
    Ignore,
}

/// 引擎上下文只读视图（暴露给中间件）。
///
/// 中间件可读取引擎级配置和统计信息，用于决策（如根据已爬取页数调整策略）。
#[derive(Debug, Clone)]
pub struct CrawlContext {
    /// Spider 名称
    pub spider_name: String,
    /// 当前抓取模式
    pub fetch_mode: FetchMode,
    /// 最大并发数
    pub max_concurrent: usize,
    /// 最大爬取页数
    pub max_pages: usize,
    /// 是否遵守 robots.txt
    pub obey_robots: bool,
    /// 已爬取页数（只读快照）
    pub pages_crawled: usize,
    /// 错误数（只读快照）
    pub errors: usize,
}

// === Middleware trait ===

/// 请求/响应中间件（Downloader Middleware 等价）。
///
/// 实现此 trait 以在请求发出前/响应返回后执行自定义逻辑。
/// 所有方法都有默认实现，只需覆盖需要的方法。
#[async_trait]
pub trait Middleware: Send + Sync {
    /// 执行优先级：越小越先执行（默认 100）。
    ///
    /// 建议约定：
    /// - 0-19：过滤类（域名/深度限制）
    /// - 20-39：请求修改类（Headers/UA/Proxy）
    /// - 40-69：响应挑战类（Cookie/CF）
    /// - 70-99：重试类（BlockedRetry/Retry）
    fn priority(&self) -> u32 { 100 }

    /// 生命周期初始化：Engine 在爬取开始前调用。
    /// 中间件可读取引擎配置、初始化内部状态（通过内部可变性，如 Mutex/AtomicUsize）。
    async fn init(&self, _ctx: &CrawlContext) {}

    /// 请求发出前拦截（可修改 headers/proxy/body）。
    async fn process_request(&self, _req: &mut SpiderRequest, _ctx: &CrawlContext) -> MwAction {
        MwAction::Continue
    }

    /// 响应返回后拦截（可修改/替换响应）。
    async fn process_response(&self, _resp: &mut SpiderResponse, _ctx: &CrawlContext) -> MwAction {
        MwAction::Continue
    }

    /// 错误处理（可决定重试或放弃）。
    async fn process_error(&self, _req: &SpiderRequest, _err: &str, _ctx: &CrawlContext) -> ErrorAction {
        ErrorAction::Propagate
    }
}

// === ItemPipeline trait ===

/// Item 管道（Item Pipeline 等价）。
///
/// 顺序处理 Spider 产出的 items：清洗 → 验证 → 去重 → 存储。
/// 返回 None 表示丢弃此 item。
///
/// # 生命周期
///
/// - `open`：爬取开始前调用，初始化资源（文件句柄、DB 连接）
/// - `process_item`：每个 item 到来时调用
/// - `close`：爬取结束后调用，释放资源（flush 缓冲、关闭连接）
#[async_trait]
pub trait ItemPipeline: Send + Sync {
    /// 生命周期：爬取开始前调用（初始化资源）。
    async fn open(&self, _ctx: &CrawlContext) {}

    /// 处理单个 item。返回 Some(item) 继续传递，None 丢弃。
    async fn process_item(&self, item: Value, _ctx: &CrawlContext) -> Option<Value>;

    /// 生命周期：爬取结束后调用（flush 缓冲、关闭连接）。
    async fn close(&self, _ctx: &CrawlContext) {}
}

// === 中间件链执行器 ===

/// 中间件链：按 priority 排序后顺序执行所有中间件。
pub(crate) struct MiddlewareChain {
    pub middlewares: Vec<Arc<dyn Middleware>>,
    pub pipelines: Vec<Arc<dyn ItemPipeline>>,
}

impl MiddlewareChain {
    pub fn new() -> Self {
        Self { middlewares: Vec::new(), pipelines: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.middlewares.is_empty() && self.pipelines.is_empty()
    }

    /// 按 priority 排序中间件（越小越先执行）。
    pub(crate) fn sort(&mut self) {
        self.middlewares.sort_by_key(|mw| mw.priority());
    }

    /// 初始化所有中间件（Engine 在爬取开始前调用）。
    pub(crate) async fn run_init(&self, ctx: &CrawlContext) {
        for mw in &self.middlewares {
            mw.init(ctx).await;
        }
    }

    /// 执行请求中间件链。返回 MwAction::Skip/Abort 时中断。
    pub(crate) async fn run_request_middlewares(
        &self,
        req: &mut SpiderRequest,
        ctx: &CrawlContext,
    ) -> MwAction {
        for mw in &self.middlewares {
            match mw.process_request(req, ctx).await {
                MwAction::Continue | MwAction::Modified => continue,
                action => return action,
            }
        }
        MwAction::Continue
    }

    /// 执行响应中间件链。
    pub(crate) async fn run_response_middlewares(
        &self,
        resp: &mut SpiderResponse,
        ctx: &CrawlContext,
    ) -> MwAction {
        for mw in &self.middlewares {
            match mw.process_response(resp, ctx).await {
                MwAction::Continue | MwAction::Modified => continue,
                action => return action,
            }
        }
        MwAction::Continue
    }

    /// 执行错误中间件链。任一返回 Retry 则重试。
    pub(crate) async fn run_error_middlewares(
        &self,
        req: &SpiderRequest,
        err: &str,
        ctx: &CrawlContext,
    ) -> ErrorAction {
        for mw in &self.middlewares {
            match mw.process_error(req, err, ctx).await {
                ErrorAction::Propagate => continue,
                action => return action,
            }
        }
        ErrorAction::Propagate
    }

    /// 执行 item 管道链。
    pub(crate) async fn run_pipelines(&self, item: Value, ctx: &CrawlContext) -> Option<Value> {
        let mut current = Some(item);
        for pipeline in &self.pipelines {
            match current {
                Some(item) => {
                    current = pipeline.process_item(item, ctx).await;
                }
                None => return None,
            }
        }
        current
    }

    /// 打开所有 pipeline（爬取开始前调用）。
    pub(crate) async fn run_pipelines_open(&self, ctx: &CrawlContext) {
        for pipeline in &self.pipelines {
            pipeline.open(ctx).await;
        }
    }

    /// 关闭所有 pipeline（爬取结束后调用）。
    pub(crate) async fn run_pipelines_close(&self, ctx: &CrawlContext) {
        for pipeline in &self.pipelines {
            pipeline.close(ctx).await;
        }
    }
}

