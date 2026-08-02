//! MCP tool gateway: protocol parsing, validation, dispatch and output shaping.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

#[cfg(feature = "stealth")]
use super::types::StealthFetchArgs;
use super::types::{AdaptiveScrapeArgs, CrawlSiteArgs, ExtractCssArgs, FetchPageArgs, ToolContext};
use crate::tools;
use wisp_core::error::{McpError, ParseError, Result, WispError};

fn parse_args<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T> {
    serde_json::from_value(args.clone()).map_err(|e| {
        WispError::Mcp(McpError::General(format!(
            "invalid arguments for {name}: {e}"
        )))
    })
}

fn to_value<T: Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value)
        .map_err(|e| WispError::Parse(ParseError::Serialize(format!("tool result serialize: {e}"))))
}

/// Dispatch one MCP tool call through the typed tool seam.
pub async fn call_tool(name: &str, args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    match name {
        "fetch_page" => {
            let args = parse_args::<FetchPageArgs>(&args, name)?;
            to_value(tools::fetch_page(args, ctx).await?)
        }
        "extract_css" => {
            let args = parse_args::<ExtractCssArgs>(&args, name)?;
            to_value(tools::extract_css(args).await?)
        }
        "crawl_site" => {
            let args = parse_args::<CrawlSiteArgs>(&args, name)?;
            to_value(tools::crawl_site(args, ctx).await?)
        }
        "adaptive_scrape" => {
            let args = parse_args::<AdaptiveScrapeArgs>(&args, name)?;
            to_value(tools::adaptive_scrape(args, ctx).await?)
        }
        #[cfg(feature = "stealth")]
        "stealth_fetch" => {
            let args = parse_args::<StealthFetchArgs>(&args, name)?;
            to_value(tools::stealth_fetch(args, ctx).await?)
        }
        _ => Err(WispError::Mcp(McpError::UnknownTool(name.into()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn extract_css_returns_texts() {
        let args = json!({
            "html": "<p class='x'>hello</p><p class='x'>world</p>",
            "selector": "p.x"
        });
        let parsed = parse_args::<ExtractCssArgs>(&args, "extract_css").unwrap();
        let result = tools::extract_css(parsed).await.unwrap();
        assert_eq!(result.texts, vec!["hello", "world"]);
        assert!(result.attrs.is_empty());
    }

    #[tokio::test]
    async fn extract_css_returns_attrs() {
        let args = json!({
            "html": "<a href='/a'>A</a>",
            "selector": "a",
            "attr": "href"
        });
        let parsed = parse_args::<ExtractCssArgs>(&args, "extract_css").unwrap();
        let result = tools::extract_css(parsed).await.unwrap();
        assert_eq!(result.attrs, vec!["/a"]);
        assert!(result.texts.is_empty());
    }

    #[test]
    fn invalid_args_are_reported_with_tool_name() {
        let err = parse_args::<FetchPageArgs>(&json!({}), "fetch_page").unwrap_err();
        assert!(err.to_string().contains("fetch_page"));
    }
}
