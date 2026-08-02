//! Engine 实现 - 从 mod.rs 拆分，降低圈复杂度。
//!
//! 核心拆解：
//! - `EngineContext` 打包单次 run 状态（替代 20+ 个 Arc 变量传递）
//! - `process_request()` 处理单个请求（替代 200 行嵌套闭包）
//! - `fetch_dispatch()` 抓取分发循环（transport 级重试 fallback）
//! - `auto_upgrade_check()` Auto 模式升级检查
//!
//! Task 3 重构：EngineContext 多 Spider 共享队列 + callback 路由，process_request
//! 调 `spider.handle()` 而非 `spider.parse()`，items 收集到 `ctx.items`。

// 注：per-domain 信号量已删除。全局并发由 buffer_unordered(buffer_ceiling) 控制，
// 动态调整由 autoscale 负责。多域名公平性由用户通过 Request::priority 或 download_delay 管理。
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Mutex;
use tracing::Instrument;

use super::stats::SpiderStats;
use super::{
    CrawlEvent, CrawlState, CrawlStats, Request, Response, Spider, auto, middleware, scheduler,
};
use crate::observability::events::EngineEvent;
use wisp_core::error::Result;
use wisp_core::utils::sanitize_url;
use wisp_fetcher::FetchMode;

// === 子模块 ===
pub(crate) mod checkpoint;
pub(crate) mod context;
pub(crate) mod fetch;
pub(crate) mod guard;
pub(crate) mod request;
pub(crate) mod response;

pub(crate) use checkpoint::{load_spider_checkpoint, persist_spider_checkpoint};
pub use context::record_status;
pub(crate) use context::{EngineContext, EngineState, build_crawl_context_for, snapshot_stats_for};
pub(crate) use fetch::fetch_dispatch;
pub use fetch::{fetch_page, fetch_page_inner};
pub(crate) use guard::InFlightGuard;
pub(crate) use request::process_request;
pub(crate) use response::process_response;
#[cfg(test)]
mod tests;
