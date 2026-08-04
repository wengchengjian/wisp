//! Spider trait、请求钩子决策与默认阻塞状态码。

use crate::{NeverStop, StopCondition};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use wisp_fetcher::{CrawlRequest, Response};

/// 请求钩子的决策结果。
#[derive(Debug, Clone, PartialEq)]
pub enum RequestAction {
    /// 正常执行
    Proceed,
    /// 跳过此请求
    Skip,
    /// 延迟指定时间后再执行
    Delay(Duration),
    /// 终止整个爬取
    Abort,
}

/// The core Spider trait users implement to define a crawler.
///
/// ND-031-ARCH 修复：Spider 只关心业务逻辑（解析什么、如何解析），
/// 引擎配置（fetch_mode/obey_robots/max_retries/download_delay/auto_rules）
/// 已迁移到 `EngineBuilder`，由 Engine 统一管理。
#[async_trait]
pub trait Spider: Send + Sync + 'static {
    // Required
    /// Spider 名称（用于日志和统计）。
    fn name(&self) -> &str;
    /// 起始 URL 列表。
    fn start_urls(&self) -> Vec<String>;
    /// 请求分发入口。Engine 调用此方法处理响应，返回 (items, follows)。
    async fn handle(&self, resp: Response) -> (Vec<Value>, Vec<CrawlRequest>);

    // Optional with defaults — 业务逻辑（保留在 Spider）
    /// 允许的域名集合（空表示不限制）。
    fn allowed_domains(&self) -> HashSet<String> {
        HashSet::new()
    }
    /// 爬取开始时的钩子。
    async fn on_start(&self) {}
    /// 爬取结束时的钩子。
    async fn on_close(&self) {}
    /// 请求失败时的钩子。
    async fn on_error(&self, _req: &CrawlRequest, _err: &str) {}
    /// Item 处理钩子（可过滤/转换）。
    async fn on_item(&self, item: Value) -> Option<Value> {
        Some(item)
    }
    /// 判断响应是否被拦截（默认检查状态码）。
    fn is_blocked(&self, resp: &Response) -> bool {
        BLOCKED_STATUS_CODES.contains(&resp.status)
    }
    /// 最大爬取深度。默认无限制。
    ///
    /// 保留在 Spider：爬取深度是业务范围决策（"我要爬多深"），非引擎行为。
    fn max_depth(&self) -> u32 {
        u32::MAX
    }
    /// 每个请求执行前的异步钩子。默认返回 Proceed。
    async fn on_before_request(&self, _req: &CrawlRequest) -> RequestAction {
        RequestAction::Proceed
    }

    // === 终止条件（保留） ===

    /// 终止条件。默认永不停止（由引擎 max_pages 兜底）。
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(NeverStop)
    }

    /// 该 Spider 是否接受指定 callback 的请求。
    ///
    /// 多 Spider 模式下，Engine 从共享队列取出请求后按此方法路由。
    /// 默认实现接受所有请求；`ClosureSpider` 根据注册的 handler 覆盖。
    fn accepts_callback(&self, _callback: Option<&str>) -> bool {
        true
    }
}

/// 默认阻塞状态码：401/403/407/429/444/500/502/503/504
pub const BLOCKED_STATUS_CODES: &[u16] = &[401, 403, 407, 429, 444, 500, 502, 503, 504];
