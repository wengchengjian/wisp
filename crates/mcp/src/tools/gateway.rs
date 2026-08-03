//! MCP tool gateway: protocol lookup and dispatch through one tool spec seam.

use serde_json::Value;

use super::types::ToolContext;
use crate::protocol::TOOLS;
use wisp_core::error::{McpError, Result, WispError};

/// Dispatch one MCP tool call through the typed tool seam.
pub async fn call_tool(name: &str, args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
    let tool = TOOLS
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| WispError::Mcp(McpError::UnknownTool(name.into())))?;
    tool.run(args, ctx).await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[tokio::test]
    async fn extract_css_returns_texts() {
        let args = json!({
            "html": "<p class='x'>hello</p><p class='x'>world</p>",
            "selector": "p.x"
        });
        let parsed =
            crate::tools::parse_args::<crate::tools::extract::ExtractCssArgs>(&args, "extract_css")
                .unwrap();
        let result = crate::tools::extract::extract_css(parsed).await.unwrap();
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
        let parsed =
            crate::tools::parse_args::<crate::tools::extract::ExtractCssArgs>(&args, "extract_css")
                .unwrap();
        let result = crate::tools::extract::extract_css(parsed).await.unwrap();
        assert_eq!(result.attrs, vec!["/a"]);
        assert!(result.texts.is_empty());
    }

    #[test]
    fn invalid_args_are_reported_with_tool_name() {
        let err = crate::tools::parse_args::<crate::tools::fetch::FetchPageArgs>(
            &json!({}),
            "fetch_page",
        )
        .unwrap_err();
        assert!(err.to_string().contains("fetch_page"));
    }
}
