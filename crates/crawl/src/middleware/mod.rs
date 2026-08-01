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
//! use wisp_crawl::middleware::{UaRotationMiddleware, RetryMiddleware};
//! use wisp_crawl::Engine;
//! use std::sync::Arc;
//!
//! let engine = Engine::infra()
//!     .ua_rotation(UaRotationMiddleware::desktop())
//!     .middleware(Arc::new(RetryMiddleware::new(
//!         std::time::Duration::from_secs(1),
//!     )))
//!     .build()
//!     .unwrap();
//! ```

pub mod builtin;
pub mod pipeline;

mod actions;
mod chain;
mod context;
mod traits;

pub use actions::*;
pub use builtin::*;
pub use context::CrawlContext;
pub use pipeline::*;
pub use traits::*;

pub(crate) use chain::MiddlewareChain;
