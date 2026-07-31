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
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)]

// 子 crate 按模块路径 re-export，保持既有 `wisp::xxx` API。
pub use wisp_browser as browser;
pub use wisp_crawl as crawl;
pub use wisp_fetcher as fetcher;
pub use wisp_http as http;
pub use wisp_mcp as mcp;
pub use wisp_parser as parser;
pub use wisp_proxy as proxy;
pub use wisp_stealth as stealth;
pub use wisp_storage as storage;
pub use wisp_core::{config, error, text, utils};

// config_file 位于 wisp-proxy crate，单独保留旧路径 `wisp::config_file`。
/// 外部 TOML 配置文件解析。
pub mod config_file {
    pub use wisp_proxy::config_file::*;
}

// === 统一入口 ===
pub use fetcher::{FetchClient, FetchClientConfig, Fetcher, FetchMode, FetcherBuilder};
pub use fetcher::{Response, Request, Method};
#[cfg(feature = "browser")]
pub use fetcher::DynamicStrategy;
#[cfg(feature = "stealth")]
pub use fetcher::StealthStrategy;
pub use fetcher::cookie::{Cookie, CookieJar, HttpCookieJar};
#[cfg(feature = "browser")]
pub use fetcher::cookie::BrowserCookieJar;
#[cfg(feature = "stealth")]
pub use fetcher::cookie::{CfCookieJar, CfSession};
pub use stealth::TurnstileConfig;

// === 核心类型 ===
pub use browser::{Browser, Page};
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{WispError, Result, BrowserError, NetworkError, ParseError, McpError, StorageError};

pub use parser::{Node, NodeList, ResponseExt};
pub use proxy::RotationStrategy;
pub use storage::{Store, MemoryStore, FileStore, CachedResponse, ElementSnapshotRow};

// 自由函数导出（业务层 API）
pub use storage::{
    save_checkpoint, load_checkpoint, delete_checkpoint,
    save_element, load_element,
    save_response, load_response, delete_response,
};

#[cfg(feature = "sqlite")]
pub use storage::SqliteStore;

// === 爬虫引擎 ===
pub use crawl::{Spider, Engine, EngineConfig, CrawlEvent, CrawlStream, Items, JsonlWriter, SpiderBuilder, ClosureSpider};
pub use http::UaRotator;

// === 底层类型（FetchClientConfig 公共字段需要） ===
pub use http::DomainBlocker;
