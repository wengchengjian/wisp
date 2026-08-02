//! MCP extract_css 工具。

use super::types::{ExtractCssArgs, ExtractCssResult};
use wisp_core::error::Result;
use wisp_parser::Node;

/// CSS 选择器提取元素。
pub async fn extract_css(args: ExtractCssArgs) -> Result<ExtractCssResult> {
    let doc = Node::from_html(&args.html);
    let nodes = doc.select(&args.selector);

    let mut result = ExtractCssResult {
        texts: Vec::new(),
        attrs: Vec::new(),
    };
    if let Some(attr) = args.attr.as_deref() {
        result.attrs = nodes
            .iter()
            .map(|n| n.attr(attr).unwrap_or_default())
            .collect();
    } else {
        result.texts = nodes.iter().map(|n| n.text()).collect();
    }
    Ok(result)
}
