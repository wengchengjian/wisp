//! MCP extract_css 工具。

use super::types::ToolContext;
use crate::protocol::{Tool, TypedRun};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use wisp_core::error::Result;
use wisp_parser::Node;

/// `extract_css` arguments.
#[derive(Debug, Deserialize)]
pub struct ExtractCssArgs {
    /// HTML source.
    pub html: String,
    /// CSS selector.
    pub selector: String,
    /// Optional attribute name to extract.
    #[serde(default)]
    pub attr: Option<String>,
}

/// `extract_css` result.
#[derive(Debug, Serialize)]
pub struct ExtractCssResult {
    /// Extracted text values.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<String>,
    /// Extracted attribute values.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attrs: Vec<String>,
}

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

fn extract_css_run<'a>(
    args: ExtractCssArgs,
    _ctx: &'a ToolContext<'a>,
) -> Pin<Box<dyn Future<Output = Result<ExtractCssResult>> + Send + 'a>> {
    Box::pin(extract_css(args))
}

pub(crate) fn spec() -> Tool {
    Tool::from_handler(
        "extract_css",
        "用 CSS 选择器从 HTML 提取元素，返回文本/属性列表。",
        json!({
            "type": "object",
            "properties": {
                "html": { "type": "string", "description": "HTML 文本" },
                "selector": { "type": "string", "description": "CSS 选择器" },
                "attr": { "type": "string", "description": "可选：提取该属性而非文本" }
            },
            "required": ["html", "selector"]
        }),
        extract_css_run as TypedRun<ExtractCssArgs, ExtractCssResult>,
    )
}
