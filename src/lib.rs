//! wisp: Lightweight undetected browser automation for Rust.
//!
//! Pure Rust CDP (Chrome DevTools Protocol) over WebSocket with anti-detection
//! patches. Built for scraping — passes Browserscan 4/4 in both headed and headless.
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use wisp::Fetcher;
//!
//! # async fn example() -> wisp::Result<()> {
//! let page = Fetcher::http().get("https://quotes.toscrape.com/").await?;
//! let quotes = page.css(".quote .text");
//! # Ok(())
//! # }
//! ```
//!
//! # Modules
//! - `fetcher` - Unified Fetcher API (Http / Dynamic / Stealth / Auto modes)
//! - `parser` - HTML parsing with CSS/XPath selectors + adaptive relocation
//! - `crawl` - Spider-based crawling engine (scheduler, checkpoint, streaming)
//! - `browser` - Core CDP browser automation (launch, page, element)
//! - `stealth` - Anti-detection patches + Cloudflare challenge solver + human simulation
//! - `http` - HTTP client with TLS fingerprint emulation (wreq)
//! - `proxy` - Proxy pool management with rotation strategies
//! - `storage` - Pluggable storage (MemoryStore + FileStore default, SqliteStore optional)
//! - `mcp` - MCP server for AI-assisted scraping (stdio JSON-RPC)
//! - `config` - Browser launch options and proxy configuration
//! - `config_file` - TOML configuration file parsing
//! - `error` - Categorized error types (Browser / Network / Parse / Mcp / Storage)
//! - `text` - Text and attribute processing utilities
//! - `utils` - Internal helpers (URL resolution, random suffix)

// ND-006-DOC：启用 missing_docs 警告（软约束）。
// 当前存在 293 个历史警告，逐步补充文档。CI 可通过 -D warnings 强制。
#![warn(missing_docs)]
// ND-014-STYLE：启用 clippy::all + pedantic（软约束）。
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
// pedantic 例外：模块名重复、问号操作符等不强制
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)]

/// 浏览器进程管理：启动 Chrome、CDP 会话、页面操作。
pub mod browser;
/// 浏览器启动选项和代理配置。
pub mod config;
/// TOML 配置文件解析。
pub mod config_file;
/// 分类错误体系（Browser / Network / Parse / Mcp / Storage）。
pub mod error;
/// 反检测补丁 + Cloudflare 挑战解决 + 人类行为模拟。
pub mod stealth;
/// 代理池管理与轮换策略。
pub mod proxy;
/// 文本和属性处理工具。
pub mod text;
/// 内部辅助工具（URL 解析、随机后缀）。
pub mod utils;
/// HTML 解析：CSS/XPath 选择器 + 自适应重定位。
pub mod parser;
/// HTTP 客户端（TLS 指纹模拟，基于 wreq）。
pub mod http;
/// 统一 Fetcher API（Http / Dynamic / Stealth / Auto 模式）。
pub mod fetcher;
/// Spider 爬虫引擎（调度器、检查点、流式处理）。
pub mod crawl;
/// 可插拔存储（MemoryStore + FileStore，可选 SqliteStore）。
pub mod storage;
/// MCP Server（AI 辅助爬取，stdio JSON-RPC）。
pub mod mcp;

// === 统一入口 ===
pub use fetcher::{FetchClient, FetchClientConfig, Fetcher, FetchMode, FetcherBuilder};
pub use fetcher::{Response, Request, Method};
pub use stealth::TurnstileConfig;

// === 核心类型 ===
pub use browser::{Browser, Page};
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{WispError, Result, BrowserError, NetworkError, ParseError, McpError, StorageError};

pub use parser::{Node, NodeList};
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
pub use crawl::{Spider, Engine, CrawlEvent, CrawlStream, Items, JsonlWriter, SpiderBuilder, ClosureSpider};
pub use http::UaRotator;

// === 底层类型（FetchClientConfig 公共字段需要） ===
pub use http::DomainBlocker;
