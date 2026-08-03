//! Middleware / ItemPipeline trait 定义。

use async_trait::async_trait;
use serde_json::Value;

use super::{CrawlContext, ErrorAction, RequestMwAction, ResponseMwAction};
use crate::{Item, Request, Response};
use wisp_core::error::WispError;

// === Middleware trait ===

/// 请求/响应中间件（Downloader Middleware 等价）。
///
/// 实现此 trait 以在请求发出前/响应返回后执行自定义逻辑。
/// 所有方法都有默认实现，只需覆盖需要的方法。
#[async_trait]
pub trait Middleware: Send + Sync {
    /// 中间件名称（默认类型名，用于诊断/观测）。
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// 执行优先级：越小越先执行（默认 100）。
    ///
    /// 建议约定：
    /// - 0-19：过滤类（robots）
    /// - 20-39：请求修改类（Headers/UA/Proxy）
    /// - 40-69：响应挑战类（Cookie/CF）
    /// - 70-99：重试类（BlockedRetry/Retry）
    fn priority(&self) -> u32 {
        100
    }

    /// 生命周期初始化：Engine 在爬取开始前调用。
    /// 中间件可读取引擎配置、初始化内部状态（通过内部可变性，如 Mutex/AtomicUsize）。
    async fn init(&self, _ctx: &CrawlContext) {}

    /// 请求发出前拦截（可修改 headers/proxy/body）。
    async fn process_request(&self, _req: &mut Request, _ctx: &CrawlContext) -> RequestMwAction {
        RequestMwAction::Continue
    }

    /// 响应返回后拦截（可修改/替换响应）。
    async fn process_response(
        &self,
        _resp: &mut Response,
        _ctx: &CrawlContext,
    ) -> ResponseMwAction {
        ResponseMwAction::Continue
    }

    /// 错误处理（可决定重试或放弃）。
    async fn process_error(
        &self,
        _req: &Request,
        _err: &WispError,
        _ctx: &CrawlContext,
    ) -> ErrorAction {
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
    async fn process_item(&self, item: Item<Value>, _ctx: &CrawlContext) -> Option<Item<Value>>;

    /// 生命周期：爬取结束后调用（flush 缓冲、关闭连接）。
    async fn close(&self, _ctx: &CrawlContext) {}
}
