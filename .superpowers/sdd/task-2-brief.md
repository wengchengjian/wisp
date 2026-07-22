# Task 2: SitemapSpider 迁移为 SpiderBuilder 预设

**Files:**
- Modify: `src/crawl/builder.rs`
- Delete: `src/crawl/templates.rs`
- Modify: `src/crawl/mod.rs`（删 `pub mod templates;`）
- New: `tests/sitemap_test.rs`

## Step 1: SpiderBuilder 加 sitemap() 预设

在 `src/crawl/builder.rs` 的 `impl SpiderBuilder` 中加：

```rust
/// 预设：Sitemap 爬虫。
///
/// 自动解析 sitemap.xml，提取 `<loc>` URL，follow 到指定 label 的 handler。
///
/// # 示例
/// ```ignore
/// let spider = SpiderBuilder::sitemap("my_spider", vec!["https://x.com/sitemap.xml".into()], "content")
///     .on("content", |resp| async move {
///         (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
///     })
///     .build();
/// ```
pub fn sitemap(name: &str, sitemap_urls: Vec<String>, content_label: &str) -> Self {
    let label = content_label.to_string();
    SpiderBuilder::new(name)
        .start_urls(sitemap_urls)
        .on("default", move |resp| {
            let label = label.clone();
            async move {
                let text = resp.text().unwrap_or_default();
                let re = regex::Regex::new(r"<loc>\s*(.*?)\s*</loc>").unwrap();
                let follows: Vec<SpiderRequest> = re.captures_iter(&text)
                    .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
                    .filter(|u| !u.is_empty())
                    .map(|url| SpiderRequest::get(&url).with_callback(&label))
                    .collect();
                (vec![], follows)
            }
        })
}
```

注意：先读取当前 `builder.rs` 确认 `on()` 方法的 handler 签名和 `SpiderRequest::get`/`with_callback`/`SpiderResponse` 的实际签名，适配上述代码使其编译通过。如果 `resp.text()` 方法不存在，改用 `String::from_utf8_lossy(&resp.body)` 或 `std::str::from_utf8(&resp.body)`。

## Step 2: 删除 templates.rs

用 DeleteFile 工具删除 `src/crawl/templates.rs`。

## Step 3: 删除 mod.rs 的 templates 声明

删除 `pub mod templates;` 行。搜索是否有 re-export 一并删除。

## Step 4: 写测试

新建 `tests/sitemap_test.rs`：

```rust
//! SpiderBuilder::sitemap() 测试。
use wisp::crawl::*;

#[test]
fn test_sitemap_builder_creates_spider() {
    let spider = SpiderBuilder::sitemap("test", vec!["https://example.com/sitemap.xml".into()], "content")
        .on("content", |_resp| async move {
            (vec![serde_json::json!({"ok": true})], vec![])
        })
        .build();
    assert_eq!(spider.name(), "test");
    assert_eq!(spider.start_urls(), vec!["https://example.com/sitemap.xml"]);
}

#[tokio::test]
async fn test_sitemap_parses_loc_urls() {
    let spider = SpiderBuilder::sitemap("test", vec!["https://example.com/sitemap.xml".into()], "content")
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
```

注意：先读取当前 SpiderResponse/SpiderRequest 的实际字段定义，适配测试构造代码。`tracker` 和 `from_cache` 是 pub 字段（#[doc(hidden)]），构造时需显式写出。

## Step 5: 验证

```
cargo build --lib
cargo test --test sitemap_test -- --nocapture
```

## Step 6: 提交

PowerShell 不支持 heredoc，用多个 -m 参数：
```
git rm src/crawl/templates.rs
git add src/crawl/builder.rs tests/sitemap_test.rs src/crawl/mod.rs
git commit -m "feat(builder): SpiderBuilder::sitemap() 预设替代 templates.rs" -m "SitemapSpider 迁移为 SpiderBuilder 预设方法" -m "删除 templates.rs 整个文件（死代码）"
```

## 背景
templates.rs 的 CrawlSpider/SitemapSpider 是死代码（全项目零引用）。SitemapSpider 迁移为 SpiderBuilder::sitemap() 预设，复用 on(label, handler) API。SpiderBuilder 已有 on() 方法和 ClosureSpider HashMap 查表分发。
