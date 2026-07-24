//! MCP 工具实现。

use serde_json::{Value, json};
use std::sync::Arc;
use crate::error::{WispError, Result};
use crate::storage::Store;
use crate::parser::Node;
use crate::http::Client;
use crate::crawl::Engine;
use wreq_util::Profile;

/// 抓取单个网页，返回 HTML 文本。
pub async fn fetch_page(args: Value) -> Result<Value> {
    let url = args.get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'url' argument".into()))?;

    let mut builder = Client::builder();
    if let Some(emu) = args.get("emulation").and_then(|v| v.as_str()) {
        // Profile 变体名查证：Firefox128/Safari18 已存在（Stage 3 验证），
        // 计划假设的 FirefoxLatest/SafariLatest 不存在，改用具体版本号。
        let profile = match emu {
            "firefox" => Profile::Firefox128,
            "safari" => Profile::Safari18,
            _ => Profile::Chrome136,
        };
        builder = builder.emulation(profile);
    }

    let client = builder.build()?;
    let resp = client.get(url, &[]).await?;
    let html = resp.text()?;

    Ok(json!({
        "url": url,
        "status": resp.status,
        "html": html,
        "bytes": resp.body.len()
    }))
}

/// CSS 选择器提取元素。
pub async fn extract_css(args: Value) -> Result<Value> {
    let html = args.get("html")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'html' argument".into()))?;
    let selector = args.get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'selector' argument".into()))?;
    let attr: Option<&str> = args.get("attr").and_then(|v| v.as_str());

    let doc = Node::from_html(html);
    let nodes = doc.select(selector);

    if let Some(a) = attr {
        let attrs: Vec<Value> = nodes.iter()
            .map(|n| json!(n.attr(a).unwrap_or_default()))
            .collect();
        Ok(json!({"attrs": attrs}))
    } else {
        let texts: Vec<Value> = nodes.iter()
            .map(|n| json!(n.text()))
            .collect();
        Ok(json!({"texts": texts}))
    }
}

/// 爬取站点：用内置 SimpleSpider 按 CSS 选择器提取，返回 JSONL。
///
/// Task 5：复用 MCP server 启动时创建的共享 Engine（HTTP 连接池 / 请求缓存 / 代理池），
/// 不再每次调用新建 Engine。per-call `max_pages` 通过 Spider 的 `until()` 终止策略生效，
/// Engine 自身的 `max_pages` 作为全局兜底。
pub async fn crawl_site(args: Value, engine: &Engine) -> Result<Value> {
    let start_urls: Vec<String> = args.get("start_urls")
        .and_then(|v| v.as_array())
        .ok_or_else(|| WispError::McpError("missing 'start_urls' array".into()))?
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    if start_urls.is_empty() {
        return Err(WispError::McpError("start_urls 不能为空".into()));
    }

    let css_selector = args.get("css_selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'css_selector'".into()))?
        .to_string();

    let max_pages = args.get("max_pages")
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as usize;

    use crate::crawl::{Spider, Request, Response, MaxPages, StopCondition};
    use async_trait::async_trait;

    struct SimpleSpider {
        css: String,
        start_urls: Vec<String>,
        max_pages: usize,
    }

    #[async_trait]
    impl Spider for SimpleSpider {
        fn name(&self) -> &str { "mcp_simple" }
        fn start_urls(&self) -> Vec<String> { self.start_urls.clone() }
        async fn parse(&self, resp: Response) -> (Vec<Value>, Vec<Request>) {
            let text = resp.text().unwrap_or_default();
            let doc = Node::from_html(&text);
            let nodes = doc.select(&self.css);
            let items: Vec<Value> = nodes.iter()
                .map(|n| json!({"text": n.text(), "html": n.html()}))
                .collect();
            (items, vec![])
        }
        fn obey_robots(&self) -> bool { false }
        // per-call max_pages：由 Spider 终止策略生效，Engine 的 max_pages 作为全局兜底
        fn until(&self) -> Arc<dyn StopCondition> {
            Arc::new(MaxPages(self.max_pages))
        }
    }

    // 复用共享 Engine，run() 返回 (stats, items)
    let spider = SimpleSpider { css: css_selector, start_urls, max_pages };
    let (_stats, items) = engine.run(spider).await?;

    let jsonl: String = items.iter()
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(json!({
        "items_count": items.len(),
        "jsonl": jsonl
    }))
}

/// 自适应抓取：CSS 失败时用 SQLite 快照重定位。
pub async fn adaptive_scrape(args: Value, store: &Arc<Store>) -> Result<Value> {
    let url = args.get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'url'".into()))?;
    let selector = args.get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'selector'".into()))?;
    let key = args.get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'key'".into()))?;

    let client = Client::builder().build()?;
    let resp = client.get(url, &[]).await?;
    let html = resp.text()?;
    let doc = Node::from_html(&html);

    use crate::parser::css_adaptive;
    let tolerance = crate::parser::DEFAULT_TOLERANCE;
    let found = css_adaptive(&doc, selector, key, url, store, true, tolerance);

    match found {
        Some(node) => Ok(json!({
            "url": url,
            "found": true,
            "text": node.text(),
            "html": node.html()
        })),
        None => Ok(json!({
            "url": url,
            "found": false
        })),
    }
}

/// 浏览器模式抓取（绕 CF Turnstile）。
pub async fn stealth_fetch(args: Value) -> Result<Value> {
    let url = args.get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'url'".into()))?;
    let headless = args.get("headless")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let human_mode = args.get("human_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    use crate::{Browser, LaunchOptions};

    let browser = Browser::launch(LaunchOptions { headless, ..Default::default() }).await
        .map_err(|e| WispError::McpError(format!("browser launch: {e}")))?;
    let mut page = browser.new_page().await
        .map_err(|e| WispError::McpError(format!("new page: {e}")))?;
    page.goto(url).await
        .map_err(|e| WispError::McpError(format!("goto: {e}")))?;

    if human_mode {
        // 人类行为模拟：随机延迟
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let html = page.evaluate_as_string("document.documentElement.outerHTML").await
        .map_err(|e| WispError::McpError(format!("get html: {e}")))?;
    let title = page.evaluate_as_string("document.title").await
        .unwrap_or_default();

    browser.close().await
        .map_err(|e| WispError::McpError(format!("close: {e}")))?;

    Ok(json!({
        "url": url,
        "title": title,
        "html": html,
        "bytes": html.len()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_extract_css_returns_text() {
        let args = json!({
            "html": "<html><body><p class='x'>hello</p><p class='x'>world</p></body></html>",
            "selector": "p.x"
        });
        let result = extract_css(args).await.unwrap();
        let texts = result["texts"].as_array().unwrap();
        assert_eq!(texts.len(), 2);
        assert_eq!(texts[0].as_str().unwrap(), "hello");
        assert_eq!(texts[1].as_str().unwrap(), "world");
    }

    #[tokio::test]
    async fn test_extract_css_returns_attr() {
        let args = json!({
            "html": "<html><body><a href='/a'>A</a><a href='/b'>B</a></body></html>",
            "selector": "a",
            "attr": "href"
        });
        let result = extract_css(args).await.unwrap();
        let attrs = result["attrs"].as_array().unwrap();
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].as_str().unwrap(), "/a");
    }

    #[tokio::test]
    async fn test_extract_css_missing_args() {
        let args = json!({});
        let result = extract_css(args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_page_missing_url() {
        let args = json!({});
        let result = fetch_page(args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_crawl_site_missing_args() {
        let engine = crate::crawl::Engine::infra()
            .max_pages(100)
            .build()
            .unwrap();
        let args = json!({});
        let result = crawl_site(args, &engine).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_adaptive_scrape_missing_args() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let args = json!({});
        let result = adaptive_scrape(args, &store).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_stealth_fetch_missing_url() {
        let args = json!({});
        let result = stealth_fetch(args).await;
        assert!(result.is_err());
    }
}
