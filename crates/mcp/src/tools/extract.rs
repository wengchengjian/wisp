//! MCP extract_css 工具。

use serde_json::{json, Value};
use wisp_core::error::{McpError, Result, WispError};
use wisp_parser::Node;

/// CSS 选择器提取元素。
pub async fn extract_css(args: Value) -> Result<Value> {
    let html = args
        .get("html")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'html' argument".into())))?;
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'selector' argument".into())))?;
    let attr: Option<&str> = args.get("attr").and_then(|v| v.as_str());

    let doc = Node::from_html(html);
    let nodes = doc.select(selector);

    if let Some(a) = attr {
        let attrs: Vec<Value> = nodes
            .iter()
            .map(|n| json!(n.attr(a).unwrap_or_default()))
            .collect();
        Ok(json!({"attrs": attrs}))
    } else {
        let texts: Vec<Value> = nodes.iter().map(|n| json!(n.text())).collect();
        Ok(json!({"texts": texts}))
    }
}
