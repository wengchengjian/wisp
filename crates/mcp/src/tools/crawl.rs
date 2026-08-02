//! MCP crawl_site 工具。

use super::types::{CrawlSiteArgs, CrawlSiteResult, ToolContext};
use regex::Regex;
use serde_json::{json, Value};
use wisp_core::error::{Result, WispError};
use wisp_crawl::{MaxPages, SpiderBuilder};

/// 爬取站点：用 SpiderBuilder + 共享 Engine 按 CSS 选择器提取，返回 JSONL。
pub async fn crawl_site(args: CrawlSiteArgs, ctx: &ToolContext<'_>) -> Result<CrawlSiteResult> {
    if args.start_urls.is_empty() {
        return Err(WispError::Config("start_urls 不能为空".into()));
    }
    for url in &args.start_urls {
        wisp_core::utils::validate_url(url)?;
    }
    let follow_pattern = args
        .follow_pattern
        .as_deref()
        .map(Regex::new)
        .transpose()
        .map_err(|e| WispError::Config(format!("invalid follow_pattern regex: {e}")))?;
    let max_depth = match args.max_depth {
        Some(d) if d > 0 => d as u32,
        _ => u32::MAX,
    };
    let max_pages = args.max_pages.unwrap_or(100).min(1000) as usize;
    let css = args.css_selector.clone();

    let mut builder = SpiderBuilder::new("mcp_simple")
        .start_urls(args.start_urls)
        .allowed_domains(args.allowed_domains.unwrap_or_default())
        .max_depth(max_depth)
        .until(MaxPages(max_pages));
    builder = builder.on_page("default", move |mut page| {
        for node in page.css(&css).iter() {
            page.item_value(json!({ "text": node.text(), "html": node.html() }));
        }
        if let Some(re) = follow_pattern.as_ref() {
            page.follow_links_filtered(
                &["a[href]"],
                "default",
                |url| re.is_match(url),
                |_page, _idx, _a| json!(null),
            );
        } else {
            page.follow_links(&["a[href]"], "default", |_page, _idx, _a| json!(null));
        }
        page
    });
    let spider = builder.build();
    let (_stats, items) = ctx.engine.run(spider).await?;
    let jsonl: String = items
        .iter()
        .map(|v: &Value| serde_json::to_string(v).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(CrawlSiteResult {
        items_count: items.len(),
        jsonl,
    })
}
