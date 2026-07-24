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
//! - `storage` - SQLite persistence (adaptive snapshots, checkpoints, cache)
//! - `mcp` - MCP server for AI-assisted scraping (stdio JSON-RPC)
//! - `config` - Browser launch options and proxy configuration
//! - `config_file` - TOML configuration file parsing
//! - `error` - Categorized error types (Browser / Network / Parse / Mcp / Storage)
//! - `text` - Text and attribute processing utilities
//! - `utils` - Internal helpers (URL resolution, random suffix)

pub mod browser;
pub mod config;
pub mod config_file;
pub mod error;
pub mod stealth;
pub mod proxy;
pub mod text;
pub mod utils;
pub mod parser;
pub mod http;
pub mod fetcher;
pub mod crawl;
pub mod storage;
pub mod mcp;

// === 统一入口 ===
pub use fetcher::{FetchClient, FetchClientConfig, Fetcher, FetchMode, FetcherBuilder};
pub use fetcher::{Response, Request, Method};

// === 核心类型 ===
pub use browser::{Browser, Page};
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{WispError, Result, BrowserError, NetworkError, ParseError, McpError, StorageError};

pub use parser::{Node, NodeList};
pub use proxy::RotationStrategy;
pub use storage::{Store, MemoryStore, SqliteStore, CachedResponse, ElementSnapshotRow};

// === 爬虫引擎 ===
pub use crawl::{Spider, Engine, CrawlEvent, CrawlStream, Items, JsonlWriter, SpiderBuilder, ClosureSpider};
pub use http::UaRotator;

// === 底层类型（FetchClientConfig 公共字段需要） ===
pub use http::DomainBlocker;
