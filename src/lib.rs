//! wisp: Lightweight undetected browser automation for Rust.
//!
//! Pure Rust CDP (Chrome DevTools Protocol) over WebSocket with anti-detection
//! patches. Built for scraping — passes Browserscan 4/4 in both headed and headless.

pub mod cdp;
pub mod browser;
pub mod element;
pub mod page;
pub mod patches;
pub mod config;
pub mod error;

pub use browser::Browser;
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{WispError, Result};
pub use page::Page;
