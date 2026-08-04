//! MCP crawl_site 工具。

use super::types::ToolContext;
use crate::protocol::{Tool, TypedRun};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use wisp_core::error::{Result, WispError};
use wisp_crawl::{Items, MaxPages, SpiderBuilder};

/// `crawl_site` arguments.
#[derive(Debug, Deserialize)]
pub struct CrawlSiteArgs {
    /// Seed URLs.
    pub start_urls: Vec<String>,
    /// CSS selector used by the built-in spider.
    pub css_selector: String,
    /// Per-call page cap.
    #[serde(default)]
    pub max_pages: Option<u64>,
    /// Optional link-following regex.
    #[serde(default)]
    pub follow_pattern: Option<String>,
    /// Maximum follow depth.
    #[serde(default)]
    pub max_depth: Option<u64>,
    /// Optional domain allowlist.
    #[serde(default)]
    pub allowed_domains: Option<Vec<String>>,
}

/// `crawl_site` result.
#[derive(Debug, Serialize)]
pub struct CrawlSiteResult {
    /// Number of items produced.
    pub items_count: usize,
    /// JSONL representation of items.
    pub jsonl: String,
}

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
    let item_count = items.len();
    let jsonl = Items::from_items(items).to_jsonl()?;
    Ok(CrawlSiteResult {
        items_count: item_count,
        jsonl,
    })
}

fn crawl_site_run<'a>(
    args: CrawlSiteArgs,
    ctx: &'a ToolContext<'a>,
) -> Pin<Box<dyn Future<Output = Result<CrawlSiteResult>> + Send + 'a>> {
    Box::pin(crawl_site(args, ctx))
}

pub(crate) fn spec() -> Tool {
    Tool::from_handler(
        "crawl_site",
        "爬取站点，返回 JSONL。用内置 SpiderBuilder 按 CSS 选择器提取。",
        json!({
            "type": "object",
            "properties": {
                "start_urls": { "type": "array", "items": { "type": "string" } },
                "css_selector": { "type": "string", "description": "每页提取的 CSS 选择器" },
                "max_pages": { "type": "integer", "default": 100 },
                "follow_pattern": { "type": "string", "description": "可选：仅跟随匹配此正则的链接" },
                "max_depth": { "type": "integer", "default": 0, "description": "最大跟随深度，0 表示不限制" },
                "allowed_domains": { "type": "array", "items": { "type": "string" }, "description": "可选：仅跟随这些域名的链接" }
            },
            "required": ["start_urls", "css_selector"]
        }),
        crawl_site_run as TypedRun<CrawlSiteArgs, CrawlSiteResult>,
    )
}
