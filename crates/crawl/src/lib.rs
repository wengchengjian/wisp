//! wisp crawl engine: multi-spider, streaming, checkpoint, middleware.

mod crawl_stream;
mod item;
mod spider;
mod stats;

pub mod adaptive;
pub(crate) mod auto;
pub mod builder;
pub mod engine;
pub mod middleware;
pub mod observability;
pub mod page;
pub mod runtime;
pub mod scenario;
pub mod scheduling;

// 兼容 re-export：保持 `wisp::crawl::stop::MaxPages` 等子模块路径可用
pub use observability::events;
pub use observability::state;
pub use runtime::autoscale;
pub use runtime::control;
pub use runtime::output;
pub use runtime::robots;
pub use scheduling::scheduler;
pub use scheduling::stop;

pub use adaptive::AdaptiveTracker;
pub use builder::{ClosureSpider, SpiderBuilder};
pub use crawl_stream::{CrawlEvent, CrawlStream};
pub use engine::{Engine, EngineBuilder, EngineConfig};
pub use item::Item;
pub use page::Page;
pub use runtime::output::{ItemOutput, Items, OutputFormat};
pub use spider::{BLOCKED_STATUS_CODES, RequestAction, Spider};
pub use stats::{CrawlState, CrawlStats};
pub use stop::{
    FnStopCondition, MaxErrors, MaxItems, MaxPages, MaxPagesByCallback, NeverStop, StopCondition,
    StopContext, Timeout, pages_by_callback,
};

// 统一类型：直接使用 fetcher 的 Request/Response/Method
pub use wisp_fetcher::{CrawlRequest, FetchClient, FetchOptions, Method, Request, Response};
// 薄壳层（mcp 等）所需底层类型：经 crawl 统一出口，避免直接依赖 fetcher/parser/storage
pub use wisp_parser::{DEFAULT_TOLERANCE, Node};
pub use wisp_storage::{open_store, Store};
pub use wreq_util::Profile;

#[cfg(test)]
mod tests;
