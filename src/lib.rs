//! wisp: Lightweight undetected browser automation for Rust.
//!
//! Pure Rust CDP (Chrome DevTools Protocol) over WebSocket with anti-detection
//! patches. Built for scraping — passes Browserscan 4/4 in both headed and headless.
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use wisp::{Fetcher, parser::ResponseExt};
//!
//! # async fn example() -> wisp::Result<()> {
//! let page = Fetcher::http().get("https://quotes.toscrape.com/").await?;
//! let quotes = page.css(".quote .text");
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![expect(clippy::doc_markdown)]

// 子 crate 按模块路径 re-export，保持既有 `wisp::xxx` API。
#[cfg(feature = "browser")]
pub use wisp_browser as browser;
pub use wisp_core::{config, error, text, utils};
pub use wisp_crawl as crawl;
pub use wisp_fetcher as fetcher;
pub use wisp_http as http;
#[cfg(feature = "mcp")]
pub use wisp_mcp as mcp;
pub use wisp_parser as parser;
pub use wisp_proxy as proxy;
#[cfg(feature = "stealth")]
pub use wisp_stealth as stealth;
pub use wisp_storage as storage;

// === 统一入口 ===
#[cfg(feature = "browser")]
pub use fetcher::DynamicStrategy;
#[cfg(feature = "stealth")]
pub use fetcher::StealthStrategy;
#[cfg(feature = "browser")]
pub use fetcher::cookie::BrowserCookieJar;
#[cfg(feature = "stealth")]
pub use fetcher::cookie::{CfCookieJar, CfSession};
pub use fetcher::cookie::{Cookie, CookieJar, HttpCookieJar};
pub use fetcher::{FetchClient, FetchClientConfig, FetchMode, Fetcher, FetcherBuilder};
pub use fetcher::{Method, Request, Response};
#[cfg(feature = "stealth")]
pub use stealth::TurnstileConfig;

// === 核心类型 ===
#[cfg(feature = "browser")]
pub use browser::{Browser, Page};
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{
    BrowserError, McpError, NetworkError, ParseError, Result, StorageError, WispError,
};

pub use parser::{Node, NodeList, ResponseExt};
pub use proxy::RotationStrategy;
pub use storage::{CachedResponse, ElementSnapshotRow, FileStore, MemoryStore, Store};

// 自由函数导出（业务层 API）
pub use storage::{
    delete_checkpoint, delete_response, load_checkpoint, load_element, load_response,
    save_checkpoint, save_element, save_response,
};

#[cfg(feature = "sqlite")]
pub use storage::SqliteStore;

// === 爬虫引擎 ===
pub use crawl::{
    ClosureSpider, CrawlEvent, CrawlStream, Engine, EngineConfig, Items, JsonlWriter, Spider,
    SpiderBuilder,
};

// === 底层类型（FetchClientConfig 公共字段需要） ===
pub use http::DomainBlocker;
