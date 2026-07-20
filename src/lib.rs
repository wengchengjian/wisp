//! # patchright-rs
//!
//! Undetected browser automation for Rust.
//! A native Rust implementation of patchright's anti-detection patches,
//! controlling Chromium directly via CDP (Chrome DevTools Protocol).
//!
//! ## Key Patches
//! - No `Runtime.enable` (uses isolated ExecutionContexts)
//! - No `Console.enable` (console disabled by design)
//! - Stealth launch args (no `--enable-automation`)
//! - Closed Shadow Root penetration
//!
//! ## Example
//! ```no_run
//! use patchright_rs::{Browser, LaunchOptions};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let browser = Browser::launch(LaunchOptions::default()).await?;
//!     let page = browser.new_page().await?;
//!     page.goto("https://example.com").await?;
//!     let webdriver = page.evaluate("navigator.webdriver").await?;
//!     assert!(webdriver.is_null());
//!     browser.close().await?;
//!     Ok(())
//! }
//! ```

pub mod browser;
pub mod cdp;
pub mod config;
pub mod element;
pub mod error;
pub mod page;
pub mod patches;

pub use browser::Browser;
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{PatchrightError, Result};
pub use page::Page;
