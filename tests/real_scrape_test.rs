//! 鐪熷疄鏁版嵁鎶撳彇娴嬭瘯濂椾欢銆?
//!
//! 杩愯鏂瑰紡锛歚cargo test --test real_scrape_test -- --ignored`
//!
//! 浣跨敤鐪熷疄缃戠珯楠岃瘉 wisp 鐨勬姄鍙栥€佽В鏋愩€乆Path銆佺紪鐮佹娴嬬瓑鍔熻兘銆?
//! 浠ｇ悊锛?27.0.0.1:7897锛堢綉缁滀笉閫氭椂鑷姩浣跨敤锛夈€?

use std::time::Duration;
use wisp::http::Client;
use wisp::parser::Node;
use wisp::crawl::{Engine, SpiderBuilder, SpiderResponse, SpiderRequest};
use wisp::proxy::RotationStrategy;
use wreq_util::Profile;
use serde_json::Value;

const PROXY: &str = "http://127.0.0.1:7897";

/// 鍒涘缓甯︿唬鐞嗙殑 client锛堢綉缁滅洿杩炲け璐ユ椂鍥為€€鍒颁唬鐞嗭級
async fn smart_client() -> Client {
    // 鍏堝皾璇曠洿杩?
    let direct = Client::builder()
        .timeout(Duration::from_secs(10))
        .emulation(Profile::Chrome136)
        .build()
        .unwrap();

    if direct.get("https://quotes.toscrape.com/").await.is_ok() {
        return direct;
    }

    // 鐩磋繛澶辫触锛屼娇鐢ㄤ唬鐞?
    Client::builder()
        .timeout(Duration::from_secs(30))
        .proxy(PROXY)
        .emulation(Profile::Chrome136)
        .build()
        .unwrap()
}

// === 娴嬭瘯 1: quotes.toscrape.com 瀹屾暣鐖彇锛堝椤碉級===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_quotes_full_crawl_10_pages() {
    let client = smart_client().await;

    // 楠岃瘉棣栭〉鍙揪
    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 涓嶅彲杈?);
        return;
    }

    let spider = SpiderBuilder::new("quotes-full")
        .start_urls(vec!["https://quotes.toscrape.com/"])
        .concurrent(4)
        .delay_ms(200)
        .obey_robots(false)
        .parse(|resp| {
            let doc = resp.parse().unwrap();
            let items: Vec<Value> = doc.select(".quote").iter().map(|q| {
                serde_json::json!({
                    "text": q.select_one(".text").map(|n| n.text()).unwrap_or_default(),
                    "author": q.select_one(".author").map(|n| n.text()).unwrap_or_default(),
                    "tags": q.select(".tag").text(),
                })
            }).collect();

            // 璺熼殢鍒嗛〉
            let follows: Vec<SpiderRequest> = doc.select_one(".next a")
                .and_then(|a| a.attr("href"))
                .and_then(|href| resp.follow(&href))
                .map(|r| vec![r])
                .unwrap_or_default();

            (items, follows)
        })
        .build();

    let stats = Engine::builder(spider)
        .max_pages(10)
        .run_one()
        .await
        .unwrap();

    assert_eq!(stats.pages_crawled, 10, "搴旂埇鍙?10 椤?);
    assert!(stats.items_scraped >= 80, "10 椤靛簲鑷冲皯 80 鏉″悕瑷€, 瀹為檯: {}", stats.items_scraped);
    assert_eq!(stats.errors, 0, "涓嶅簲鏈夐敊璇?);
    println!("瀹屾暣鐖彇: {}", stats.summary());
}

// === 娴嬭瘯 2: books.toscrape.com 涔︾睄淇℃伅鎻愬彇 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_books_toscrape_extraction() {
    let client = smart_client().await;

    let resp = client.get("https://books.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: books.toscrape.com 涓嶅彲杈?);
        return;
    }
    let resp = resp.unwrap();
    let doc = resp.parse().unwrap();

    // 鎻愬彇涔︾睄淇℃伅
    let books = doc.select("article.product_pod");
    assert!(books.len() >= 10, "棣栭〉搴旀湁鑷冲皯 10 鏈功, 瀹為檯: {}", books.len());

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

        assert!(!title.is_empty(), "涔﹀悕涓嶅簲涓虹┖");
        assert!(price.contains('拢') || price.contains("脗拢"), "浠锋牸搴斿惈 拢 绗﹀彿: {}", price);
        assert!(rating.contains("star-rating"), "搴旀湁璇勫垎 class: {}", rating);
    }

    println!("PASS: 鎴愬姛鎻愬彇 {} 鏈功鐨勪俊鎭?, books.len());
}

// === 娴嬭瘯 3: XPath 澶嶆潅琛ㄨ揪寮忕湡瀹為〉闈㈡祴璇?===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_xpath_real_page() {
    let client = smart_client().await;

    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 涓嶅彲杈?);
        return;
    }
    let resp = resp.unwrap();
    let doc = resp.parse().unwrap();

    // 绠€鍗?XPath
    let quotes = doc.xpath("//div[@class='quote']");
    assert!(quotes.len() >= 5, "XPath 搴旀壘鍒拌嚦灏?5 涓?quote div");

    // 灞炴€ч€夋嫨
    let authors = doc.xpath("//small[@class='author']");
    assert!(authors.len() >= 5, "XPath 搴旀壘鍒拌嚦灏?5 涓綔鑰?);

    // 楠岃瘉 CSS 鍜?XPath 缁撴灉涓€鑷?
    let css_quotes = doc.select(".quote");
    assert_eq!(css_quotes.len(), quotes.len(), "CSS 鍜?XPath 缁撴灉鏁伴噺搴斾竴鑷?);

    println!("PASS: XPath 鐪熷疄椤甸潰娴嬭瘯閫氳繃 ({} quotes)", quotes.len());
}

// === 娴嬭瘯 4: find_by_text 鐪熷疄椤甸潰娴嬭瘯 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_find_by_text_real_page() {
    let client = smart_client().await;

    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 涓嶅彲杈?);
        return;
    }
    let resp = resp.unwrap();
    let doc = resp.parse().unwrap();

    // 鎸夋枃鏈煡鎵句綔鑰?
    let albert = doc.find_by_text("Albert Einstein", Some("small"), true);
    assert!(!albert.is_empty(), "搴旀壘鍒?Albert Einstein 鐨勫厓绱?);

    // 鍖呭惈鍖归厤
    let einstein_mentions = doc.find_by_text("Einstein", None, false);
    assert!(!einstein_mentions.is_empty(), "搴旀壘鍒板惈 Einstein 鏂囨湰鐨勫厓绱?);

    println!("PASS: find_by_text 鐪熷疄椤甸潰娴嬭瘯閫氳繃");
}

// === 娴嬭瘯 5: find_similar 鐪熷疄椤甸潰娴嬭瘯 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_find_similar_real_page() {
    let client = smart_client().await;

    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 涓嶅彲杈?);
        return;
    }
    let resp = resp.unwrap();
    let doc = resp.parse().unwrap();

    // 鑾峰彇绗竴涓?quote 鍏冪礌锛屾煡鎵剧浉浼煎厓绱?
    let first_quote = doc.select_one(".quote").expect("搴旀湁 quote 鍏冪礌");
    let similar = first_quote.find_similar();

    // 椤甸潰涓婃湁澶氫釜 .quote锛宖ind_similar 搴旀壘鍒板叾浠?quote
    assert!(
        similar.len() >= 3,
        "搴旀壘鍒拌嚦灏?3 涓浉浼煎厓绱? 瀹為檯: {}",
        similar.len()
    );

    println!("PASS: find_similar 鎵惧埌 {} 涓浉浼煎厓绱?, similar.len());
}

// === 娴嬭瘯 6: response.follow() 渚挎嵎鏂规硶娴嬭瘯 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_response_follow_pagination() {
    let client = smart_client().await;

    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 涓嶅彲杈?);
        return;
    }
    let fetch_resp = resp.unwrap();
    let doc = fetch_resp.parse().unwrap();

    // 鏋勯€?SpiderResponse 鏉ユ祴璇?follow()
    let spider_resp = SpiderResponse {
        url: "https://quotes.toscrape.com/".into(),
        status: 200,
        headers: Default::default(),
        body: fetch_resp.body.clone(),
        request: SpiderRequest::get("https://quotes.toscrape.com/"),
    };

    // 鑾峰彇涓嬩竴椤甸摼鎺?
    let next_href = doc.select_one(".next a")
        .and_then(|a| a.attr("href"));

    if let Some(href) = next_href {
        let follow_req = spider_resp.follow(&href);
        assert!(follow_req.is_some(), "follow() 搴旇繑鍥?Some");
        let req = follow_req.unwrap();
        assert!(req.url.starts_with("https://quotes.toscrape.com/"), "URL 搴斾负缁濆璺緞: {}", req.url);
        println!("PASS: follow() 鐢熸垚 URL: {}", req.url);
    } else {
        eprintln!("WARN: 鏈壘鍒颁笅涓€椤甸摼鎺?);
    }
}

// === 娴嬭瘯 7: 缂栫爜妫€娴嬫祴璇?===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_encoding_detection() {
    let client = smart_client().await;

    // quotes.toscrape.com 鏄?UTF-8
    let resp = client.get("https://quotes.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: quotes.toscrape.com 涓嶅彲杈?);
        return;
    }
    let resp = resp.unwrap();
    let text = resp.text().unwrap();

    // 楠岃瘉 UTF-8 鍐呭姝ｇ‘瑙ｇ爜锛堝惈鐗规畩瀛楃锛?
    assert!(text.contains("鈥?) || text.contains("\""), "搴旀纭В鐮?UTF-8 寮曞彿");
    assert!(!text.contains("茂驴陆"), "涓嶅簲鏈?UTF-8 瑙ｇ爜澶辫触鏍囪");

    println!("PASS: 缂栫爜妫€娴嬫甯?);
}

// === 娴嬭瘯 8: SpiderBuilder + Engine 鑱斿悎娴嬭瘯 ===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_spider_builder_engine_integration() {
    let client = smart_client().await;
    let resp = client.get("https://books.toscrape.com/").await;
    if resp.is_err() {
        eprintln!("SKIP: books.toscrape.com 涓嶅彲杈?);
        return;
    }

    let spider = SpiderBuilder::new("books")
        .start_urls(vec!["https://books.toscrape.com/"])
        .concurrent(2)
        .delay_ms(300)
        .obey_robots(false)
        .max_retries(2)
        .parse(|resp| {
            let doc = resp.parse().unwrap();
            let items: Vec<Value> = doc.select("article.product_pod").iter().map(|book| {
                serde_json::json!({
                    "title": book.select_one("h3 a").and_then(|a| a.attr("title")).unwrap_or_default(),
                    "price": book.select_one(".price_color").map(|n| n.text()).unwrap_or_default(),
                })
            }).collect();

            // 璺熼殢涓嬩竴椤?
            let follows: Vec<SpiderRequest> = doc.select_one("li.next a")
                .and_then(|a| a.attr("href"))
                .and_then(|href| resp.follow(&href))
                .map(|r| vec![r])
                .unwrap_or_default();

            (items, follows)
        })
        .build();

    let stats = Engine::builder(spider)
        .max_pages(3)
        .run_one()
        .await
        .unwrap();

    assert_eq!(stats.pages_crawled, 3, "搴旂埇鍙?3 椤?);
    assert!(stats.items_scraped >= 40, "3 椤靛簲鑷冲皯 40 鏈功, 瀹為檯: {}", stats.items_scraped);
    println!("Books 鐖彇: {}", stats.summary());
}
