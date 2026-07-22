//! 真实数据抓取测试套件。
//!
//! 运行方式：`cargo test --test real_scrape_test -- --ignored`
//!
//! 使用真实网站验证 wisp 的抓取、解析、XPath、编码检测等功能。
//! 代理：127.0.0.1:7897（网络不通时自动使用）。

use std::time::Duration;
use wisp::http::Client;
use wisp::parser::Node;
use wisp::crawl::{Engine, SpiderBuilder, SpiderResponse, SpiderRequest};
use wisp::proxy::RotationStrategy;
use wreq_util::Profile;
use serde_json::Value;

const PROXY: &str = "http://127.0.0.1:7897";

/// 创建带代理的 client（网络直连失败时回退到代理）
async fn smart_client() -> Client {
    // 先尝试直连
    let direct = Client::builder()
        .timeout(Duration::from_secs(10))
        .emulation(Profile::Chrome136)
        .build()
        .unwrap();

    if direct.get("https://quotes.toscrape.com/").await.is_ok() {
        return direct;
    }

    // 直连失败，使用代理
    Client::builder()
        .timeout(Duration::from_secs(30))
        .proxy(PROXY)
        .emulation(Profile::Chrome136)
        .build()
        .unwrap()
}

// === 测试 1: quotes.toscrape.com 完整抓取（多页）===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_quotes_full_crawl_10_pages() {
    let client = smart_client().await;

    // 验证首页可达
    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 不可达");
        return;
    }

    let spider = SpiderBuilder::new("quotes-full")
        .start_urls(vec!["https://quotes.toscrape.com/"])
        .concurrent(4)
        .delay_ms(200)
        .obey_robots(false)
        .on("default", |resp| async move {
            let doc = resp.parse().unwrap();
            let items: Vec<Value> = doc.select(".quote").iter().map(|q| {
                serde_json::json!({
                    "text": q.select_one(".text").map(|n| n.text()).unwrap_or_default(),
                    "author": q.select_one(".author").map(|n| n.text()).unwrap_or_default(),
                    "tags": q.select(".tag").text(),
                })
            }).collect();

            // 跟踪分页
            let follows: Vec<SpiderRequest> = doc.select_one(".next a")
                .and_then(|a| a.attr("href"))
                .and_then(|href| resp.follow(&href))
                .map(|r| vec![r])
                .unwrap_or_default();

            (items, follows)
        })
        .build();

    let engine = Engine::infra().max_pages(10).build().unwrap();
    let (stats, _items) = engine.run(spider).await.unwrap();

    assert_eq!(stats.pages_crawled, 10, "应爬取 10 页");
    assert!(stats.items_scraped >= 80, "10 页应至少 80 条名言, 实际: {}", stats.items_scraped);
    assert_eq!(stats.errors, 0, "不应有错误");
    println!("完整抓取: {}", stats.summary());
}

// === 测试 2: books.toscrape.com 书籍信息提取 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_books_toscrape_extraction() {
    let client = smart_client().await;

    let resp = client.get("https://books.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: books.toscrape.com 不可达");
        return;
    }
    let resp = resp.unwrap();
    let doc = resp.parse().unwrap();

    // 提取书籍信息
    let books = doc.select("article.product_pod");
    assert!(books.len() >= 10, "首页应至少 10 本书, 实际: {}", books.len());

    for book in books.iter() {
        let title = book.select_one("h3 a")
            .and_then(|a| a.attr("title"))
            .unwrap_or_default();
        let price = book.select_one(".price_color")
            .map(|n| n.text())
            .unwrap_or_default();
        let rating = book.select_one("p.star-rating")
            .and_then(|n| n.attr("class"))
            .unwrap_or_default();

        assert!(!title.is_empty(), "书名不应为空");
        assert!(price.contains('£'), "价格应含 £ 符号: {}", price);
        assert!(rating.contains("star-rating"), "应有评分 class: {}", rating);
    }

    println!("PASS: 成功提取 {} 本书的信息", books.len());
}

// === 测试 3: XPath 复杂表达式真实页面测试 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_xpath_real_page() {
    let client = smart_client().await;

    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 不可达");
        return;
    }
    let resp = resp.unwrap();
    let doc = resp.parse().unwrap();

    // 简单 XPath
    let quotes = doc.xpath("//div[@class='quote']");
    assert!(quotes.len() >= 5, "XPath 应找到至少 5 个 quote div");

    // 属性选择
    let authors = doc.xpath("//small[@class='author']");
    assert!(authors.len() >= 5, "XPath 应找到至少 5 个作者");

    // 验证 CSS 和 XPath 结果一致
    let css_quotes = doc.select(".quote");
    assert_eq!(css_quotes.len(), quotes.len(), "CSS 和 XPath 结果数量应一致");

    println!("PASS: XPath 真实页面测试通过 ({} quotes)", quotes.len());
}

// === 测试 4: find_by_text 真实页面测试 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_find_by_text_real_page() {
    let client = smart_client().await;

    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 不可达");
        return;
    }
    let resp = resp.unwrap();
    let doc = resp.parse().unwrap();

    // 按文本查找作者
    let albert = doc.find_by_text("Albert Einstein", Some("small"), true);
    assert!(!albert.is_empty(), "应找到 Albert Einstein 的元素");

    // 包含匹配
    let einstein_mentions = doc.find_by_text("Einstein", None, false);
    assert!(!einstein_mentions.is_empty(), "应找到含 Einstein 文本的元素");

    println!("PASS: find_by_text 真实页面测试通过");
}

// === 测试 5: find_similar 真实页面测试 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_find_similar_real_page() {
    let client = smart_client().await;

    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 不可达");
        return;
    }
    let resp = resp.unwrap();
    let doc = resp.parse().unwrap();

    // 获取第一个 quote 元素，查找相似元素
    let first_quote = doc.select_one(".quote").expect("应有 quote 元素");
    let similar = first_quote.find_similar();

    // 页面上有多个 .quote，find_similar 应找到其他 quote
    assert!(
        similar.len() >= 3,
        "应找到至少 3 个相似元素, 实际: {}",
        similar.len()
    );

    println!("PASS: find_similar 找到 {} 个相似元素", similar.len());
}

// === 测试 6: response.follow() 便捷方法测试 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_response_follow_pagination() {
    let client = smart_client().await;

    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 不可达");
        return;
    }
    let fetch_resp = resp.unwrap();
    let doc = fetch_resp.parse().unwrap();

    // 构造 SpiderResponse 来测试 follow()
    let spider_resp = SpiderResponse {
        url: "https://quotes.toscrape.com/".into(),
        status: 200,
        headers: Default::default(),
        body: fetch_resp.body.clone(),
        request: SpiderRequest::get("https://quotes.toscrape.com/"),
        tracker: None,
        from_cache: false,
    };

    // 获取下一页链接
    let next_href = doc.select_one(".next a")
        .and_then(|a| a.attr("href"));

    if let Some(href) = next_href {
        let follow_req = spider_resp.follow(&href);
        assert!(follow_req.is_some(), "follow() 应返回 Some");
        let req = follow_req.unwrap();
        assert!(req.url.starts_with("https://quotes.toscrape.com/"), "URL 应为绝对路径: {}", req.url);
        println!("PASS: follow() 生成 URL: {}", req.url);
    } else {
        eprintln!("WARN: 未找到下一页链接");
    }
}

// === 测试 7: 编码检测测试 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_encoding_detection() {
    let client = smart_client().await;

    // quotes.toscrape.com 是 UTF-8
    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 不可达");
        return;
    }
    let resp = resp.unwrap();
    let text = resp.text().unwrap();

    // 验证 UTF-8 内容正确解码（含特殊字符）
    assert!(text.contains('\u{201C}') || text.contains("\""), "应正确解码 UTF-8 引号");
    assert!(!text.contains('\u{FFFD}'), "不应有 UTF-8 解码失败标记");

    println!("PASS: 编码检测正常");
}

// === 测试 8: SpiderBuilder + Engine 联合测试 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_spider_builder_engine_integration() {
    let client = smart_client().await;
    let resp = client.get("https://books.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: books.toscrape.com 不可达");
        return;
    }

    let spider = SpiderBuilder::new("books")
        .start_urls(vec!["https://books.toscrape.com/"])
        .concurrent(2)
        .delay_ms(300)
        .obey_robots(false)
        .max_retries(2)
        .on("default", |resp| async move {
            let doc = resp.parse().unwrap();
            let items: Vec<Value> = doc.select("article.product_pod").iter().map(|book| {
                serde_json::json!({
                    "title": book.select_one("h3 a").and_then(|a| a.attr("title")).unwrap_or_default(),
                    "price": book.select_one(".price_color").map(|n| n.text()).unwrap_or_default(),
                })
            }).collect();

            // 跟踪下一页
            let follows: Vec<SpiderRequest> = doc.select_one("li.next a")
                .and_then(|a| a.attr("href"))
                .and_then(|href| resp.follow(&href))
                .map(|r| vec![r])
                .unwrap_or_default();

            (items, follows)
        })
        .build();

    let engine = Engine::infra().max_pages(3).build().unwrap();
    let (stats, _items) = engine.run(spider).await.unwrap();

    assert_eq!(stats.pages_crawled, 3, "应爬取 3 页");
    assert!(stats.items_scraped >= 40, "3 页应至少 40 本书, 实际: {}", stats.items_scraped);
    println!("Books 抓取: {}", stats.summary());
}
