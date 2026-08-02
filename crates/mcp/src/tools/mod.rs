//! MCP 工具实现。

mod adaptive;
mod crawl;
mod extract;
mod fetch;
mod gateway;
#[cfg(feature = "stealth")]
mod stealth;
mod types;

pub use adaptive::adaptive_scrape;
pub use crawl::crawl_site;
pub use extract::extract_css;
pub use fetch::fetch_page;
pub use gateway::call_tool;
#[cfg(feature = "stealth")]
pub use stealth::stealth_fetch;
pub use types::ToolContext;
