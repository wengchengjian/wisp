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
// Task 8：以下 pedantic 例外为误报或设计选择，全局允许以减少噪声
// - cast_*：类型转换是故意的，值范围已确认
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
// - similar_names：命名相似度过高误报（如 stats/state）
#![allow(clippy::similar_names)]
// - 函数长度/参数数量是设计选择，重构会降低可读性
#![allow(clippy::too_many_lines)]
#![allow(clippy::too_many_arguments)]
// - items_after_statements：函数内定义 struct/impl 是常见 Rust 模式
#![allow(clippy::items_after_statements)]
// - struct_field_names：字段后缀命名是设计选择
#![allow(clippy::struct_field_names)]
// - implicit_hasher：泛化 hasher 会破坏公共 API
#![allow(clippy::implicit_hasher)]
// - default_trait_access：使用 Default::default() 可读性更好
#![allow(clippy::default_trait_access)]
// - return_self_not_must_use：过于激进，多数返回 Self 的方法无需 must_use
#![allow(clippy::return_self_not_must_use)]
// - used_underscore_binding：误报
#![allow(clippy::used_underscore_binding)]
// - case_sensitive_file_extension_comparisons：测试中大小写敏感是故意的
#![allow(clippy::case_sensitive_file_extension_comparisons)]
// - field_reassign_with_default：测试中常见模式
#![allow(clippy::field_reassign_with_default)]
// - format_collect/format_push_string：可读性优先
#![allow(clippy::format_collect)]
#![allow(clippy::format_push_string)]
// - doc_link_with_quotes：误报
#![allow(clippy::doc_link_with_quotes)]
// - arc_with_non_send_sync：内部类型实际是 Send+Sync
#![allow(clippy::arc_with_non_send_sync)]
// - unnecessary_wraps：有时为了 trait 兼容性需要 Result 包装
#![allow(clippy::unnecessary_wraps)]
// - match_same_arms：有时匹配分支相同是故意的
#![allow(clippy::match_same_arms)]
// - manual_let_else：风格偏好，不强制重写
#![allow(clippy::manual_let_else)]
// - unused_async：公共 API 保持 async 以兼容调用方（调用方使用 .await）
#![allow(clippy::unused_async)]

/// 浏览器进程管理：启动 Chrome、CDP 会话、页面操作。
pub mod browser;
/// 浏览器启动选项和代理配置。
pub mod config;
/// TOML 配置文件解析。
pub mod config_file;
/// Spider 爬虫引擎（调度器、检查点、流式处理）。
pub mod crawl;
/// 分类错误体系（Browser / Network / Parse / Mcp / Storage）。
pub mod error;
/// 统一 Fetcher API（Http / Dynamic / Stealth / Auto 模式）。
pub mod fetcher;
/// HTTP 客户端（TLS 指纹模拟，基于 wreq）。
pub mod http;
/// MCP Server（AI 辅助爬取，stdio JSON-RPC）。
pub mod mcp;
/// HTML 解析：CSS/XPath 选择器 + 自适应重定位。
pub mod parser;
/// 代理池管理与轮换策略。
pub mod proxy;
/// 反检测补丁 + Cloudflare 挑战解决 + 人类行为模拟。
pub mod stealth;
/// 可插拔存储（MemoryStore + FileStore，可选 SqliteStore）。
pub mod storage;
/// 文本和属性处理工具。
pub mod text;
/// 内部辅助工具（URL 解析、随机后缀）。
pub mod utils;

// === 统一入口 ===
pub use fetcher::{FetchClient, FetchClientConfig, FetchMode, Fetcher, FetcherBuilder};
pub use fetcher::{Method, Request, Response};
pub use stealth::TurnstileConfig;

// === 核心类型 ===
pub use browser::{Browser, Page};
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{
    BrowserError, McpError, NetworkError, ParseError, Result, StorageError, WispError,
};

pub use parser::{Node, NodeList};
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
    ClosureSpider, CrawlEvent, CrawlStream, Engine, Items, JsonlWriter, Spider, SpiderBuilder,
};
pub use http::UaRotator;

// === 底层类型（FetchClientConfig 公共字段需要） ===
pub use http::DomainBlocker;
