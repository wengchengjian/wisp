//! 真实环境端到端爬虫测试套件。
//!
//! 运行方式：`cargo test --test crawl_e2e_real_test -- --ignored`
//!
//! 所有测试均标注 `#[ignore]`，避免 CI 因网络问题失败。
//! 手动运行时需保证网络可访问以下站点：
//! - httpbin.org — HTTP 测试服务（状态码、HTML 等）
//! - quotes.toscrape.com — 专为爬虫练习设计的站点（分页、CSS 选择器）
//! - example.com — 简单 HTML 页面（域名过滤测试）
//!
//! 覆盖 8 个场景：单页抓取 + CSS 提取 / 名言抓取 / 分页跟随 /
//! allowed_domains 过滤 / 503 重试 / 流式事件 / JSONL 导出 / 缓存 replay。

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use wisp::crawl::{
    CrawlEvent, CrawlStats, Engine, JsonlWriter, Spider, SpiderRequest, SpiderResponse,
};
use wisp::http::Client;
use wisp::storage::Store;

/// 探测 httpbin.org 是否可达且未被 Cloudflare 拦截。
///
/// 背景：httpbin.org 自 2025 年起间歇性被 Cloudflare 503 拦截，
/// 即使是 wisp 内置的 Chrome 136 TLS 指纹也无法绕过。
/// 此函数用于在依赖 httpbin.org 的测试前做可达性检查，
/// 不可达时通过 `eprintln` 输出原因并让测试 graceful 跳过（return），
/// 避免 CI 因外部站点不可达而失败。
async fn httpbin_reachable() -> bool {
    let client = match Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    match client.get("https://httpbin.org/status/200").await {
        Ok(r) => r.status == 200,
        Err(_) => false,
    }
}

// === 测试 1: 单页抓取 + CSS 提取（httpbin.org/html） ===

struct HttpbinHtmlSpider;

#[async_trait]
impl Spider for HttpbinHtmlSpider {
    fn name(&self) -> &str {
        "e2e-httpbin-html"
    }
    fn start_urls(&self) -> Vec<String> {
        vec!["https://httpbin.org/html".to_string()]
    }
    fn obey_robots(&self) -> bool {
        false
    }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let node = resp.parse().unwrap();
        let h1_texts: Vec<String> = node.select("h1").text();
        // httpbin.org/html 的 h1 是 "Herman Melville - Moby Dick"
        let combined = h1_texts.join(" ");
        assert!(
            combined.contains("Herman Melville"),
            "h1 应含 'Herman Melville', 实际: {}",
            combined
        );
        let items: Vec<Value> = h1_texts
            .into_iter()
            .filter(|t| !t.is_empty())
            .map(|t| serde_json::json!({ "h1": t }))
            .collect();
        (items, vec![])
    }
}

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_e2e_fetch_single_page_httpbin() {
    if !httpbin_reachable().await {
        eprintln!(
            "跳过 test_e2e_fetch_single_page_httpbin: httpbin.org 不可达（可能被 Cloudflare 拦截）"
        );
        return;
    }
    let stats = Engine::new(HttpbinHtmlSpider)
        .max_pages(1)
        .run()
        .await
        .unwrap();
    assert_eq!(stats.pages_crawled, 1, "应抓取 1 页");
    assert!(
        stats.items_scraped >= 1,
        "应至少提取 1 个 item, 实际: {}",
        stats.items_scraped
    );
    assert!(
        stats.status_code_counts.get(&200).is_some(),
        "应有 200 状态码统计: {:?}",
        stats.status_code_counts
    );
}

// === 测试 2: quotes.toscrape.com CSS 提取 ===

struct QuotesSpider;

#[async_trait]
impl Spider for QuotesSpider {
    fn name(&self) -> &str {
        "e2e-quotes"
    }
    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }
    fn obey_robots(&self) -> bool {
        false
    }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let node = resp.parse().unwrap();
        let quotes = node.select(".quote");
        let items: Vec<Value> = quotes
            .iter()
            .map(|q| {
                let text = q.select_one(".text").map(|n| n.text()).unwrap_or_default();
                let author = q
                    .select_one(".author")
                    .map(|n| n.text())
                    .unwrap_or_default();
                serde_json::json!({ "text": text, "author": author })
            })
            .collect();
        (items, vec![])
    }
}

#[tokio::test]
#[ignore = "requires network access to quotes.toscrape.com"]
async fn test_e2e_crawl_quotes_toscrape() {
    let stats = Engine::new(QuotesSpider).max_pages(1).run().await.unwrap();
    assert_eq!(stats.pages_crawled, 1, "应抓取 1 页");
    assert!(
        stats.items_scraped >= 5,
        "首页应有至少 5 条名言, 实际: {}",
        stats.items_scraped
    );
}

// === 测试 3: 跟随分页链接 ===

struct QuotesFollowSpider;

#[async_trait]
impl Spider for QuotesFollowSpider {
    fn name(&self) -> &str {
        "e2e-quotes-follow"
    }
    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }
    fn obey_robots(&self) -> bool {
        false
    }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let node = resp.parse().unwrap();
        let quotes = node.select(".quote");
        let items: Vec<Value> = quotes
            .iter()
            .map(|q| {
                let text = q.select_one(".text").map(|n| n.text()).unwrap_or_default();
                serde_json::json!({ "text": text })
            })
            .collect();
        // 跟随 .next a 分页链接（相对路径，需补全域名）
        let follows: Vec<SpiderRequest> = node
            .select_one(".next a")
            .and_then(|a| a.attr("href"))
            .map(|href| {
                let url = format!("https://quotes.toscrape.com{}", href);
                vec![SpiderRequest::get(&url)]
            })
            .unwrap_or_default();
        (items, follows)
    }
}

#[tokio::test]
#[ignore = "requires network access to quotes.toscrape.com"]
async fn test_e2e_follow_links_quotes_toscrape() {
    let stats = Engine::new(QuotesFollowSpider)
        .max_pages(3)
        .run()
        .await
        .unwrap();
    assert_eq!(
        stats.pages_crawled, 3,
        "应抓取 3 页, 实际: {}",
        stats.pages_crawled
    );
    assert!(
        stats.items_scraped >= 20,
        "3 页应至少 20 条名言, 实际: {}",
        stats.items_scraped
    );
}

// === 测试 4: allowed_domains 过滤 ===

struct DomainFilterSpider;

#[async_trait]
impl Spider for DomainFilterSpider {
    fn name(&self) -> &str {
        "e2e-domain-filter"
    }
    fn start_urls(&self) -> Vec<String> {
        // start_urls 含 example.com，但 allowed_domains 只允许 quotes.toscrape.com
        vec!["https://example.com/".to_string()]
    }
    fn allowed_domains(&self) -> HashSet<String> {
        let mut s = HashSet::new();
        s.insert("quotes.toscrape.com".to_string());
        s
    }
    fn obey_robots(&self) -> bool {
        false
    }
    async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        (vec![], vec![])
    }
}

#[tokio::test]
#[ignore = "requires network access"]
async fn test_e2e_allowed_domains_filter() {
    let stats = Engine::new(DomainFilterSpider)
        .max_pages(5)
        .run()
        .await
        .unwrap();
    assert_eq!(stats.pages_crawled, 0, "example.com 应被过滤");
    assert!(
        stats.offsite_requests_count >= 1,
        "应统计 offsite 请求, 实际: {}",
        stats.offsite_requests_count
    );
}

// === 测试 5: 重试机制（503） ===

struct Retry503Spider;

#[async_trait]
impl Spider for Retry503Spider {
    fn name(&self) -> &str {
        "e2e-retry-503"
    }
    fn start_urls(&self) -> Vec<String> {
        vec!["https://httpbin.org/status/503".to_string()]
    }
    fn obey_robots(&self) -> bool {
        false
    }
    fn max_retries(&self) -> u32 {
        2
    }
    fn download_delay(&self) -> std::time::Duration {
        std::time::Duration::from_millis(100)
    }
    async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        (vec![], vec![])
    }
}

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_e2e_retry_on_blocked_status() {
    let stats = Engine::new(Retry503Spider)
        .max_pages(1)
        .run()
        .await
        .unwrap();
    assert_eq!(stats.pages_crawled, 0, "503 不应计入成功页");
    assert!(stats.errors >= 1, "应有错误统计, 实际: {}", stats.errors);
    assert!(
        stats.retry_count >= 2,
        "应重试至少 2 次, 实际: {}",
        stats.retry_count
    );
    assert!(
        stats.blocked_requests >= 3,
        "blocked 请求应 >= 3 (初始 + 2 次重试), 实际: {}",
        stats.blocked_requests
    );
    let count_503 = stats.status_code_counts.get(&503).copied().unwrap_or(0);
    assert!(
        count_503 >= 3,
        "应有 >= 3 次 503 状态码, 实际: {}",
        count_503
    );
}

// === 测试 6: 流式事件 ===

struct StreamQuotesSpider;

#[async_trait]
impl Spider for StreamQuotesSpider {
    fn name(&self) -> &str {
        "e2e-stream"
    }
    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }
    fn obey_robots(&self) -> bool {
        false
    }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let node = resp.parse().unwrap();
        let items: Vec<Value> = node
            .select(".quote")
            .iter()
            .map(|q| {
                let text = q.select_one(".text").map(|n| n.text()).unwrap_or_default();
                serde_json::json!({ "text": text })
            })
            .collect();
        (items, vec![])
    }
}

#[tokio::test]
#[ignore = "requires network access to quotes.toscrape.com"]
async fn test_e2e_streaming_events() {
    let engine = Engine::new(StreamQuotesSpider).max_pages(1);
    let mut stream = engine.stream().events();
    let mut item_count = 0;
    let mut page_scraped_count = 0;
    let mut done_count = 0;
    let mut done_stats: Option<CrawlStats> = None;
    while let Some(event) = stream.next().await {
        match event {
            CrawlEvent::Item(_) => item_count += 1,
            CrawlEvent::PageScraped { .. } => page_scraped_count += 1,
            CrawlEvent::Done(stats) => {
                done_count += 1;
                done_stats = Some(stats);
                break;
            }
            _ => {}
        }
    }
    assert!(
        item_count >= 1,
        "应至少收到 1 个 Item 事件, 实际: {}",
        item_count
    );
    assert!(
        page_scraped_count >= 1,
        "应至少收到 1 个 PageScraped 事件, 实际: {}",
        page_scraped_count
    );
    assert_eq!(done_count, 1, "应收到 1 个 Done 事件");
    let stats = done_stats.expect("应收到 Done 事件携带 stats");
    assert!(
        stats.pages_crawled >= 1,
        "Done 事件 stats 应含 pages_crawled >= 1, 实际: {}",
        stats.pages_crawled
    );
}

// === 测试 7: JSONL 流式导出 ===

#[tokio::test]
#[ignore = "requires network access to quotes.toscrape.com"]
async fn test_e2e_jsonl_export() {
    let path = std::env::temp_dir().join("wisp_e2e_test.jsonl");
    // 清理可能的旧文件
    let _ = std::fs::remove_file(&path);

    let engine = Engine::new(QuotesSpider).max_pages(1);
    let mut items_stream = engine.stream().items();
    let mut writer = JsonlWriter::new(&path).unwrap();
    let mut count = 0;
    while let Some(item) = items_stream.next().await {
        writer.write(&item).unwrap();
        count += 1;
    }
    writer.flush().unwrap();
    assert!(count >= 5, "应至少写入 5 条 item, 实际: {}", count);

    // 验证文件存在且行数 >= 5
    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.trim_end().lines().collect();
    assert!(
        lines.len() >= 5,
        "文件应有 >= 5 行, 实际: {}",
        lines.len()
    );
    // 每行应是合法 JSON
    for (i, line) in lines.iter().enumerate() {
        let _: Value = serde_json::from_str(line).unwrap_or_else(|e| {
            panic!("第 {} 行不是合法 JSON: {} - {}", i + 1, e, line)
        });
    }

    let _ = std::fs::remove_file(&path);
}

// === 测试 8: development_mode 缓存 replay ===

struct CacheSpider;

#[async_trait]
impl Spider for CacheSpider {
    fn name(&self) -> &str {
        "e2e-cache"
    }
    fn start_urls(&self) -> Vec<String> {
        vec!["https://httpbin.org/get".to_string()]
    }
    fn obey_robots(&self) -> bool {
        false
    }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let text = resp.text().unwrap_or_default();
        assert!(text.contains("httpbin.org"), "响应应来自 httpbin");
        (vec![], vec![])
    }
}

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_e2e_development_mode_cache_replay() {
    if !httpbin_reachable().await {
        eprintln!(
            "跳过 test_e2e_development_mode_cache_replay: httpbin.org 不可达（可能被 Cloudflare 拦截）"
        );
        return;
    }
    let store = Arc::new(Store::open_in_memory().unwrap());

    // 第一次运行：发网络请求，保存缓存
    let stats1 = Engine::new(CacheSpider)
        .max_pages(1)
        .development_mode(store.clone())
        .run()
        .await
        .unwrap();
    assert_eq!(stats1.pages_crawled, 1, "第一次应抓取 1 页");
    assert_eq!(stats1.cache_hits, 0, "第一次无命中");

    // 验证缓存已保存
    let cached = store
        .load_cached_response("https://httpbin.org/get", "GET")
        .unwrap();
    assert!(cached.is_some(), "响应应已缓存");

    // 第二次运行：命中缓存
    let stats2 = Engine::new(CacheSpider)
        .max_pages(1)
        .development_mode(store.clone())
        .run()
        .await
        .unwrap();
    assert_eq!(stats2.pages_crawled, 1, "第二次应抓取 1 页");
    assert_eq!(stats2.cache_hits, 1, "第二次应命中缓存, 实际: {}", stats2.cache_hits);
}
