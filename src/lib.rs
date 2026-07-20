//! wisp: Lightweight undetected browser automation for Rust.
//!
//! Pure Rust CDP (Chrome DevTools Protocol) over WebSocket with anti-detection
//! patches. Built for scraping — passes Browserscan 4/4 in both headed and headless.
//!
//! # Modules
//! - `browser` / `page` - Core CDP browser automation
//! - `challenge` - Cloudflare challenge detection & auto-solve
//! - `human` - Human behavior simulation (mouse, scroll, typing)
//! - `proxy` - Proxy pool management with rotation
//! - `scraper` - High-level scraping API with automatic CF bypass

pub mod cdp;
pub mod browser;
pub mod element;
pub mod page;
pub mod patches;
pub mod config;
pub mod error;
pub mod challenge;
pub mod human;
pub mod proxy;
pub mod scraper;

pub use browser::Browser;
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{WispError, Result};
pub use page::Page;
pub use scraper::{Scraper, ScrapeResponse};
