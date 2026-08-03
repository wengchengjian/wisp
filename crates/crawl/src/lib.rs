//! wisp crawl engine: multi-spider, streaming, checkpoint, middleware.

mod crawl_stats;
mod crawl_stream;
mod spider;

pub mod adaptive;
pub(crate) mod auto;
pub mod builder;
pub mod engine;
pub mod middleware;
pub mod observability;
pub mod page;
pub mod runtime;
pub mod scheduling;

// 兼容 re-export：保持 `wisp::crawl::stop::MaxPages` 等子模块路径可用
pub use observability::events;
pub use observability::state;
pub use observability::stats;
pub use runtime::autoscale;
pub use runtime::control;
pub use runtime::items;
pub use runtime::output;
pub use runtime::robots;
pub use scheduling::scheduler;
pub use scheduling::stop;

pub use adaptive::AdaptiveTracker;
pub use builder::{ClosureSpider, SpiderBuilder};
pub use crawl_stats::CrawlStats;
pub use crawl_stream::{CrawlEvent, CrawlStream};
pub use engine::{Engine, EngineBuilder, EngineConfig};
pub use items::{Items, JsonlWriter};
pub use page::Page;
pub use spider::{BLOCKED_STATUS_CODES, RequestAction, Spider};
pub use state::CrawlState;
pub use stop::{
    FnStopCondition, MaxErrors, MaxItems, MaxPages, MaxPagesByCallback, NeverStop, StopCondition,
    StopContext, Timeout, pages_by_callback,
};

pub use self::stats::SpiderStats;

// 统一类型：直接使用 fetcher 的 Request/Response/Method
pub use wisp_fetcher::{Method, Request, Response};

#[cfg(test)]
mod tests;
