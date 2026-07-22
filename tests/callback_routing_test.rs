//! callback label 路由测试。
//!
//! 验证 ClosureSpider 的 `handle()` 方法根据 `resp.request.callback` 字段
//! 路由到对应 handler 的逻辑。不依赖真实 HTTP 请求，直接构造 SpiderResponse。

use wisp::crawl::{Spider, SpiderBuilder, SpiderRequest, SpiderResponse};
use wisp::crawl::stop::MaxPages;
use serde_json::{json, Value};
use std::collections::HashMap;

/// 构造测试用 SpiderResponse。
fn make_resp(url: &str, body: &str, callback: Option<&str>) -> SpiderResponse {
    let mut req = SpiderRequest::get(url);
    if let Some(cb) = callback {
        req = req.with_callback(cb);
    }
    SpiderResponse {
        url: url.to_string(),
        status: 200,
        headers: HashMap::new(),
        body: body.as_bytes().to_vec(),
        request: req,
        tracker: None,
        from_cache: false,
    }
}

#[tokio::test]
async fn test_callback_routes_default_when_no_callback() {
    // callback=None → "default" handler
    let spider = SpiderBuilder::new("route")
        .start_urls(vec!["https://example.com/"])
        .on("default", |_resp| async move {
            (vec![json!({"stage": "list"})], vec![])
        })
        .on("detail", |_resp| async move {
            (vec![json!({"stage": "detail"})], vec![])
        })
        .build();

    let resp = make_resp("https://example.com/", "<html></html>", None);
    let (items, follows) = spider.handle(resp).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["stage"], "list");
    assert!(follows.is_empty());
}

#[tokio::test]
async fn test_callback_routes_detail_label() {
    // callback="detail" → detail handler
    let spider = SpiderBuilder::new("route")
        .start_urls(vec!["https://example.com/"])
        .on("default", |_resp| async move {
            (vec![json!({"stage": "list"})], vec![])
        })
        .on("detail", |_resp| async move {
            (vec![json!({"stage": "detail"})], vec![])
        })
        .build();

    let resp = make_resp("https://example.com/detail/1", "<html></html>", Some("detail"));
    let (items, _) = spider.handle(resp).await;
    assert_eq!(items[0]["stage"], "detail");
}

#[tokio::test]
async fn test_callback_routes_content_label_extracts_data() {
    // callback="content" → content handler 提取数据
    let spider = SpiderBuilder::new("route")
        .start_urls(vec!["https://example.com/"])
        .on("content", |resp| async move {
            let title = resp.css("h1").text().join("");
            (vec![json!({"stage": "content", "title": title})], vec![])
        })
        .build();

    let resp = make_resp(
        "https://example.com/content/1",
        "<html><body><h1>文章标题</h1></body></html>",
        Some("content"),
    );
    let (items, _) = spider.handle(resp).await;
    assert_eq!(items[0]["stage"], "content");
    assert_eq!(items[0]["title"], "文章标题");
}

#[tokio::test]
async fn test_callback_unknown_label_falls_back_to_default() {
    // callback="unknown" → 无匹配 handler → 回退到 "default"
    let spider = SpiderBuilder::new("route")
        .start_urls(vec!["https://example.com/"])
        .on("default", |_resp| async move {
            (vec![json!({"fallback": true})], vec![])
        })
        .on("detail", |_resp| async move {
            (vec![json!({"fallback": false})], vec![])
        })
        .build();

    let resp = make_resp("https://example.com/unknown", "<html></html>", Some("unknown"));
    let (items, _) = spider.handle(resp).await;
    assert_eq!(items[0]["fallback"], true);
}

#[tokio::test]
async fn test_callback_default_label_explicit_string() {
    // callback="default"（显式字符串）→ 等价于 None，走 default handler
    let spider = SpiderBuilder::new("route")
        .start_urls(vec!["https://example.com/"])
        .on("default", |_resp| async move {
            (vec![json!({"hit": "default"})], vec![])
        })
        .build();

    let resp = make_resp("https://example.com/", "<html></html>", Some("default"));
    let (items, _) = spider.handle(resp).await;
    assert_eq!(items[0]["hit"], "default");
}

#[tokio::test]
async fn test_callback_default_handler_serves_no_callback() {
    // 只注册 "default" handler，无 callback 时走 default handler
    let spider = SpiderBuilder::new("fallback")
        .start_urls(vec!["https://example.com/"])
        .on("default", |_resp| async move {
            (vec![json!({"via": "default"})], vec![])
        })
        .build();

    let resp = make_resp("https://example.com/", "<html></html>", None);
    let (items, _) = spider.handle(resp).await;
    assert_eq!(items[0]["via"], "default");
}

#[tokio::test]
async fn test_callback_pipeline_produces_follows() {
    // 验证 default handler 产出带 callback label 的 follow 请求
    let spider = SpiderBuilder::new("pipeline")
        .start_urls(vec!["https://example.com/list"])
        .on("default", |resp| async move {
            // 列表页：follow 到 "detail"
            let follows: Vec<_> = resp.css("a").iter()
                .filter_map(|a| {
                    a.attr("href").and_then(|h| resp.follow_with(&h, "detail"))
                })
                .collect();
            (vec![], follows)
        })
        .on("detail", |_resp| async move {
            (vec![json!({"stage": "detail"})], vec![])
        })
        .until(MaxPages(100))
        .build();

    let resp = make_resp(
        "https://example.com/list",
        r#"<html><body>
            <a href="/detail/1">详情1</a>
            <a href="/detail/2">详情2</a>
        </body></html>"#,
        None,
    );
    let (items, follows) = spider.handle(resp).await;
    assert!(items.is_empty());
    assert_eq!(follows.len(), 2, "应产出 2 个 detail follow 请求");
    // 每个 follow 都带 callback="detail"
    for f in &follows {
        assert_eq!(f.callback.as_deref(), Some("detail"));
    }
    // 验证 follow 的 URL 正确解析为绝对路径
    assert!(follows.iter().any(|f| f.url == "https://example.com/detail/1"));
    assert!(follows.iter().any(|f| f.url == "https://example.com/detail/2"));

    // 用其中一个 follow 构造响应，验证 detail handler 被调用
    let detail_resp = make_resp(
        "https://example.com/detail/1",
        "<html></html>",
        Some("detail"),
    );
    let (items, _) = spider.handle(detail_resp).await;
    assert_eq!(items[0]["stage"], "detail");
}

#[tokio::test]
async fn test_spider_trait_default_handle_calls_parse() {
    // 验证 Spider trait 的默认 handle() 实现调用 parse()
    use async_trait::async_trait;

    struct PlainSpider;
    #[async_trait]
    impl Spider for PlainSpider {
        fn name(&self) -> &str { "plain" }
        fn start_urls(&self) -> Vec<String> { vec![] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            (vec![json!({"default_handle": true})], vec![])
        }
    }

    let spider = PlainSpider;
    let resp = make_resp("https://example.com/", "<html></html>", None);
    // 默认 handle() 应调用 parse()
    let (items, _) = spider.handle(resp).await;
    assert_eq!(items[0]["default_handle"], true);
}

#[tokio::test]
async fn test_callback_empty_handler_returns_empty() {
    // 只注册 on()，无 "default" handler，无 callback 匹配时返回空
    let spider = SpiderBuilder::new("empty")
        .start_urls(vec!["https://example.com/"])
        .on("only", |_resp| async move {
            (vec![json!({"hit": "only"})], vec![])
        })
        .build();

    // callback=None，无 "default" handler → 返回空
    let resp = make_resp("https://example.com/", "<html></html>", None);
    let (items, follows) = spider.handle(resp).await;
    assert!(items.is_empty());
    assert!(follows.is_empty());

    // callback="only" → 命中 handler
    let resp2 = make_resp("https://example.com/", "<html></html>", Some("only"));
    let (items, _) = spider.handle(resp2).await;
    assert_eq!(items[0]["hit"], "only");
}
