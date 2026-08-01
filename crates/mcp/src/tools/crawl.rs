//! MCP crawl_site 工具。

use serde_json::{json, Value};
use wisp_core::error::{McpError, Result, WispError};
use wisp_crawl::Engine;

use super::spider::SimpleSpider;
use regex::Regex;

/// 爬取站点：用内置 SimpleSpider 按 CSS 选择器提取，返回 JSONL。
///
/// Task 5：复用 MCP server 启动时创建的共享 Engine（HTTP 连接池 / 请求缓存 / 代理池），
/// 不再每次调用新建 Engine。per-call `max_pages` 通过 Spider 的 `until()` 终止策略生效，
/// Engine 自身的 `max_pages` 作为全局兜底。
pub async fn crawl_site(args: Value, engine: &Engine) -> Result<Value> {
    let start_urls: Vec<String> = args
        .get("start_urls")
        .and_then(|v| v.as_array())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'start_urls' array".into())))?
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    if start_urls.is_empty() {
        return Err(WispError::Mcp(McpError::General(
            "start_urls 不能为空".into(),
        )));
    }
    for url in &start_urls {
        wisp_core::utils::validate_url(url)?;
    }
    let css_selector = args
        .get("css_selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'css_selector'".into())))?
        .to_string();
    let max_pages = args
        .get("max_pages")
        .and_then(|v| v.as_u64())
        .unwrap_or(100)
        .min(1000) as usize;
    let follow_pattern = args
        .get("follow_pattern")
        .and_then(|v| v.as_str())
        .map(|p| {
            Regex::new(p).map_err(|e| {
                WispError::Mcp(McpError::General(format!(
                    "invalid follow_pattern regex: {e}"
                )))
            })
        })
        .transpose()?;
    let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let allowed_domains: Vec<String> = args
        .get("allowed_domains")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let spider = SimpleSpider {
        css: css_selector,
        start_urls,
        max_pages,
        follow_pattern,
        max_depth,
        allowed_domains,
    };
    let (_stats, items) = engine.run(spider).await?;
    let jsonl: String = items
        .iter()
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(json!({
        "items_count": items.len(),
        "jsonl": jsonl
    }))
}
