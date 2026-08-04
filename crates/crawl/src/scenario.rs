//! 编排门面：组合 fetch + parse + storage，为 mcp 等壳层提供高层场景函数。
//!
//! 壳层（如 mcp）只依赖本模块，不直接依赖 fetcher/parser/storage。
//! ADR-0018：所有资源访问统一经 [`ScenarioContext`]，保持单一资源边界。

use std::sync::Arc;

use wisp_core::FetchMode;
use wisp_core::error::{Result, WispError};
use wisp_fetcher::{FetchClient, FetchOptions, Request};
use wisp_parser::{DEFAULT_TOLERANCE, Node};
use wisp_storage::{Store, open_store};

use crate::{AdaptiveTracker, Engine, Items, MaxPages, SpiderBuilder};

/// 共享资源上下文：壳层创建的编排上下文。
pub struct ScenarioContext<'a> {
    /// Persistence store for checkpoint/adaptive snapshot tools.
    pub store: &'a Arc<dyn Store>,
    /// Shared crawl Engine.
    pub engine: &'a Engine,
    /// Shared FetchClient for HTTP/browser/stealth transports.
    pub fetch_client: &'a Arc<FetchClient>,
}

impl ScenarioContext<'_> {
    /// 获取持久化 store：优先使用指定的 `path`（非空时打开），否则回退到共享 `store`。
    ///
    /// 路径为空时直接返回共享 store，避免重复打开。
    pub fn get_or_open_store(&self, path: Option<&str>) -> Result<Arc<dyn Store>> {
        match path {
            Some(p) if !p.is_empty() => open_store(p),
            _ => Ok(Arc::clone(self.store)),
        }
    }
}

/// 抓取并解码后的单页结果。
pub struct FetchedPage {
    /// Final URL.
    pub url: String,
    /// HTTP status code.
    pub status: u16,
    /// Page title.
    pub title: Option<String>,
    /// Decoded HTML.
    pub html: String,
    /// Raw body byte count.
    pub bytes: usize,
}

/// 校验 URL、按模式抓取并统一解码 HTML。
pub async fn fetch_page_html(
    ctx: &ScenarioContext<'_>,
    url: &str,
    mode: FetchMode,
    options: &FetchOptions,
) -> Result<FetchedPage> {
    wisp_core::utils::validate_url(url)?;
    let resp = ctx
        .fetch_client
        .fetch_with(&Request::get(url), mode, options)
        .await?;
    let html = resp.text()?;
    Ok(FetchedPage {
        url: resp.url,
        status: resp.status,
        title: resp.title,
        html,
        bytes: resp.body.len(),
    })
}

/// CSS 选择器提取结果。
pub struct ExtractResult {
    /// Extracted text values.
    pub texts: Vec<String>,
    /// Extracted attribute values.
    pub attrs: Vec<String>,
}

/// 用 CSS 选择器从 HTML 提取元素，返回文本/属性列表。
pub fn extract_css(html: &str, selector: &str, attr: Option<&str>) -> Result<ExtractResult> {
    let doc = Node::from_html(html);
    let nodes = doc.select(selector);
    let mut result = ExtractResult {
        texts: Vec::new(),
        attrs: Vec::new(),
    };
    if let Some(attr) = attr {
        result.attrs = nodes
            .iter()
            .map(|n| n.attr(attr).unwrap_or_default())
            .collect();
    } else {
        result.texts = nodes.iter().map(|n| n.text()).collect();
    }
    Ok(result)
}

/// 自适应抓取结果。
pub struct ScrapeResult {
    /// Target URL.
    pub url: String,
    /// Whether the element was found.
    pub found: bool,
    /// Extracted text.
    pub text: Option<String>,
    /// Extracted HTML.
    pub html: Option<String>,
}

/// 自适应抓取：CSS 失败时用快照存储重定位。
pub async fn adaptive_scrape(
    ctx: &ScenarioContext<'_>,
    url: &str,
    selector: &str,
    key: &str,
    db_path: Option<&str>,
) -> Result<ScrapeResult> {
    let page = fetch_page_html(ctx, url, FetchMode::Http, &FetchOptions::default()).await?;
    let doc = Node::from_html(&page.html);
    let effective_store = ctx.get_or_open_store(db_path)?;
    let tracker = AdaptiveTracker::new(effective_store);
    let found = tracker
        .css_adaptive(&doc, selector, key, &page.url, true, DEFAULT_TOLERANCE)
        .await?;
    match found {
        Some(node) => Ok(ScrapeResult {
            url: page.url,
            found: true,
            text: Some(node.text()),
            html: Some(node.html()),
        }),
        None => Ok(ScrapeResult {
            url: page.url,
            found: false,
            text: None,
            html: None,
        }),
    }
}

/// 站点爬取选项。
pub struct CrawlSiteOpts {
    /// Seed URLs.
    pub start_urls: Vec<String>,
    /// CSS selector used by the built-in spider.
    pub css_selector: String,
    /// Per-call page cap.
    pub max_pages: Option<u64>,
    /// Optional link-following regex.
    pub follow_pattern: Option<String>,
    /// Maximum follow depth.
    pub max_depth: Option<u64>,
    /// Optional domain allowlist.
    pub allowed_domains: Option<Vec<String>>,
}

/// 站点爬取结果。
pub struct CrawlSiteResult {
    /// Number of items produced.
    pub items_count: usize,
    /// JSONL representation of items.
    pub jsonl: String,
}

/// 爬取站点：用 SpiderBuilder + 共享 Engine 按 CSS 选择器提取，返回 JSONL。
pub async fn crawl_site_by_css(
    ctx: &ScenarioContext<'_>,
    opts: CrawlSiteOpts,
) -> Result<CrawlSiteResult> {
    if opts.start_urls.is_empty() {
        return Err(WispError::Config("start_urls 不能为空".into()));
    }
    for url in &opts.start_urls {
        wisp_core::utils::validate_url(url)?;
    }
    let follow_pattern = opts
        .follow_pattern
        .as_deref()
        .map(regex::Regex::new)
        .transpose()
        .map_err(|e| WispError::Config(format!("invalid follow_pattern regex: {e}")))?;
    let max_depth = match opts.max_depth {
        Some(d) if d > 0 => d as u32,
        _ => u32::MAX,
    };
    let max_pages = opts.max_pages.unwrap_or(100).min(1000) as usize;
    let css = opts.css_selector;

    let mut builder = SpiderBuilder::new("mcp_simple")
        .start_urls(opts.start_urls)
        .allowed_domains(opts.allowed_domains.unwrap_or_default())
        .max_depth(max_depth)
        .until(MaxPages(max_pages));
    builder = builder.on_page("default", move |mut page| {
        for node in page.css(&css).iter() {
            page.item_value(serde_json::json!({ "text": node.text(), "html": node.html() }));
        }
        if let Some(re) = follow_pattern.as_ref() {
            page.follow_links_filtered(
                &["a[href]"],
                "default",
                |url| re.is_match(url),
                |_page, _idx, _a| serde_json::json!(null),
            );
        } else {
            page.follow_links(&["a[href]"], "default", |_page, _idx, _a| {
                serde_json::json!(null)
            });
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
