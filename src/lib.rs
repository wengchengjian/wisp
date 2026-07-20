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
