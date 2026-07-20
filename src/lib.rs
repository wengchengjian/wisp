pub mod cdp;
pub mod browser;
pub mod page;
pub mod patches;
pub mod config;
pub mod error;

pub use browser::Browser;
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{PatchrightError, Result};
pub use page::Page;
