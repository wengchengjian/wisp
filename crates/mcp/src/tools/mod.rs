//! MCP 工具实现。

pub(crate) mod crawl;
pub(crate) mod extract;
pub(crate) mod fetch;
pub(crate) mod gateway;
#[cfg(feature = "stealth")]
pub(crate) mod stealth;
mod types;

pub use crawl::crawl_site;
pub use extract::extract_css;
pub use fetch::fetch_page;
pub use gateway::call_tool;
#[cfg(feature = "stealth")]
pub use stealth::stealth_fetch;
pub use types::ToolContext;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use wisp_core::error::{McpError, ParseError, Result, WispError};

pub(crate) fn parse_args<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T> {
    serde_json::from_value(args.clone()).map_err(|e| {
        WispError::Mcp(McpError::General(format!(
            "invalid arguments for {name}: {e}"
        )))
    })
}

pub(crate) fn to_value<T: Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value)
        .map_err(|e| WispError::Parse(ParseError::Serialize(format!("tool result serialize: {e}"))))
}
