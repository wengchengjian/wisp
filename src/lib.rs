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
//! - `parser` - HTML parsing with CSS/XPath selectors
//! - `crawl` - Spider-based crawling engine
//! - `browser` / `page` - Core CDP browser automation
//! - `challenge` - Cloudflare challenge detection & auto-solve
//! - `human` - Human behavior simulation
//! - `proxy` - Proxy pool management with rotation
//! - `fetch` - HTTP client internals
//! - `text` - Text and attribute processing

pub mod browser;
pub mod config;
pub mod error;
pub mod stealth;
pub mod proxy;
pub mod text;
pub mod parser;
pub mod http;
pub mod fetcher;
pub mod crawl;
pub mod storage;
pub mod mcp;

// === 统一入口 ===
pub use fetcher::{Fetcher, FetchMode, FetcherConfig, FetcherBuilder};
pub use fetcher::{Response, Request, Method, Session};

// === 核心类型 ===
pub use browser::{Browser, Page};
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{WispError, Result};

pub use parser::{Node, NodeList};
pub use proxy::RotationStrategy;
pub use storage::Store;

// === 爬虫引擎 ===
pub use crawl::{Spider, Engine, CrawlEvent, CrawlStream, Items, JsonlWriter, SpiderBuilder, ClosureSpider, SessionManager, FetcherType, RequestCache};
pub use http::UaRotator;

// === 兼容层 ===
pub use http::{Client, HttpSession, DomainBlocker};
