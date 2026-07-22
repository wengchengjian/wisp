//! SpiderBuilder::sitemap() 测试。
use wisp::crawl::*;

#[test]
fn test_sitemap_builder_creates_spider() {
    let spider = SpiderBuilder::sitemap(
        "test",
        vec!["https://example.com/sitemap.xml".into()],
        "content",
    )
    .on("content", |_resp| async move {
        (vec![serde_json::json!({"ok": true})], vec![])
    })
    .build();
    assert_eq!(spider.name(), "test");
    assert_eq!(
        spider.start_urls(),
        vec!["https://example.com/sitemap.xml"]
    );
}

#[tokio::test]
async fn test_sitemap_parses_loc_urls() {
    let spider = SpiderBuilder::sitemap(
        "test",
        vec!["https://example.com/sitemap.xml".into()],
        "content",
    )
    .on("content", |_resp| async move { (vec![], vec![]) })
    .build();

    let sitemap_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset>
  <url><loc>https://example.com/page1</loc></url>
  <url><loc>https://example.com/page2</loc></url>
</urlset>"#;
    let resp = SpiderResponse {
        url: "https://example.com/sitemap.xml".into(),
        status: 200,
        headers: Default::default(),
        body: sitemap_xml.as_bytes().to_vec(),
        request: SpiderRequest::get("https://example.com/sitemap.xml"),
        tracker: None,
        from_cache: false,
    };

    let (items, follows) = spider.handle(resp).await;
    assert!(items.is_empty());
    assert_eq!(follows.len(), 2);
    assert_eq!(follows[0].url, "https://example.com/page1");
    assert_eq!(follows[1].url, "https://example.com/page2");
    assert_eq!(follows[0].callback.as_deref(), Some("content"));
}
