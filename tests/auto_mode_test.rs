//! Auto 妯″紡娴嬭瘯锛歎RL 娉涘寲銆佽鍒欏紩鎿庛€侀€夋嫨鍣ㄨ拷韪€佹嫤鎴娴嬨€?

use std::collections::HashSet;
use wisp::crawl::auto::{generalize_url, ModeRuleEngine, SelectorTracker, is_blocked_response};
use wisp::FetchMode;
use std::collections::HashMap;

// === URL 娉涘寲娴嬭瘯 ===

#[test]
fn test_generalize_numeric_id() {
    assert_eq!(generalize_url("https://shop.com/products/123"), "/products/\\d+");
    assert_eq!(generalize_url("https://shop.com/page/2"), "/page/\\d+");
    assert_eq!(generalize_url("https://shop.com/item/99999/reviews"), "/item/\\d+/reviews");
}

#[test]
fn test_generalize_uuid() {
    assert_eq!(
        generalize_url("https://shop.com/item/deadbeef-cafe-1234-5678"),
        "/item/[a-f0-9-]+"
    );
    assert_eq!(
        generalize_url("https://api.io/v1/a1b2c3d4e5f6"),
        "/v1/[a-f0-9-]+"
    );
}

#[test]
fn test_generalize_static_path() {
    assert_eq!(generalize_url("https://shop.com/about"), "/about");
    assert_eq!(generalize_url("https://shop.com/products/list"), "/products/list");
    assert_eq!(generalize_url("https://shop.com/"), "/");
}

#[test]
fn test_generalize_mixed() {
    assert_eq!(generalize_url("https://shop.com/user/42/posts"), "/user/\\d+/posts");
    assert_eq!(generalize_url("https://shop.com/api/v2/items/7"), "/api/v2/items/\\d+");
}

// === 瑙勫垯寮曟搸娴嬭瘯 ===

#[test]
fn test_user_rule_priority() {
    let mut engine = ModeRuleEngine::new();
    engine.add_user_rule(r"/api/.*", FetchMode::Http).unwrap();
    // 鑷姩瑙勫垯璇?/api/data 闇€瑕?Dynamic
    engine.learn("https://shop.com/api/data", FetchMode::Dynamic);

    // 鐢ㄦ埛瑙勫垯浼樺厛
    assert_eq!(engine.resolve("https://shop.com/api/data"), Some(FetchMode::Http));
}

#[test]
fn test_auto_rule_matches_similar_urls() {
    let mut engine = ModeRuleEngine::new();
    engine.learn("https://shop.com/products/1", FetchMode::Dynamic);

    // 鍚屾ā鏉?URL 搴斿懡涓?
    assert_eq!(engine.resolve("https://shop.com/products/2"), Some(FetchMode::Dynamic));
    assert_eq!(engine.resolve("https://shop.com/products/999"), Some(FetchMode::Dynamic));
}

#[test]
fn test_no_rule_returns_none() {
    let engine = ModeRuleEngine::new();
    assert_eq!(engine.resolve("https://shop.com/unknown/page"), None);
}

#[test]
fn test_learn_updates_existing_pattern() {
    let mut engine = ModeRuleEngine::new();
    engine.learn("https://shop.com/products/1", FetchMode::Dynamic);
    engine.learn("https://shop.com/products/2", FetchMode::Stealth);

    // 涓嶆柊澧烇紝鏇存柊
    assert_eq!(engine.auto_rule_count(), 1);
    assert_eq!(engine.resolve("https://shop.com/products/3"), Some(FetchMode::Stealth));
}

#[test]
fn test_multiple_patterns_coexist() {
    let mut engine = ModeRuleEngine::new();
    engine.learn("https://shop.com/products/1", FetchMode::Dynamic);
    engine.learn("https://shop.com/blog/hello-world", FetchMode::Http);

    assert_eq!(engine.resolve("https://shop.com/products/5"), Some(FetchMode::Dynamic));
    // /blog/hello-world 娉涘寲鍚庢槸 /blog/hello-world (瀛楅潰閲?锛屼笉鍖归厤鍏朵粬 blog
    assert_eq!(engine.resolve("https://shop.com/blog/hello-world"), Some(FetchMode::Http));
}

// === 閫夋嫨鍣ㄨ拷韪祴璇?===

#[test]
fn test_tracker_zero_match_triggers_upgrade() {
    let mut tracker = SelectorTracker::new();
    tracker.record(".product-card", 0);
    tracker.record(".header", 1);

    assert!(tracker.needs_upgrade(&HashSet::new()));
}

#[test]
fn test_tracker_exclude_respected() {
    let mut tracker = SelectorTracker::new();
    tracker.record(".cookie-banner", 0);
    tracker.record(".product-card", 5);

    let mut exclude = HashSet::new();
    exclude.insert(".cookie-banner".to_string());

    assert!(!tracker.needs_upgrade(&exclude));
}

#[test]
fn test_tracker_all_matched_no_upgrade() {
    let mut tracker = SelectorTracker::new();
    tracker.record(".product-card", 10);
    tracker.record(".price", 10);
    tracker.record("h1", 1);

    assert!(!tracker.needs_upgrade(&HashSet::new()));
}

#[test]
fn test_tracker_empty_records_no_upgrade() {
    let tracker = SelectorTracker::new();
    assert!(!tracker.needs_upgrade(&HashSet::new()));
}

// === 鎷︽埅妫€娴嬫祴璇?===

#[test]
fn test_blocked_status_codes() {
    assert!(is_blocked_response(403, b"", &HashMap::new()));
    assert!(is_blocked_response(429, b"", &HashMap::new()));
    assert!(is_blocked_response(503, b"", &HashMap::new()));
    assert!(!is_blocked_response(200, b"ok", &HashMap::new()));
    assert!(!is_blocked_response(404, b"not found", &HashMap::new()));
}

#[test]
fn test_blocked_cf_challenge_in_body() {
    assert!(is_blocked_response(200, b"<title>Just a moment...</title>", &HashMap::new()));
    assert!(is_blocked_response(200, b"<div id='cf-challenge-running'>", &HashMap::new()));
    assert!(is_blocked_response(200, b"challenge-platform/h/b", &HashMap::new()));
    assert!(is_blocked_response(200, b"Attention Required", &HashMap::new()));
    assert!(is_blocked_response(200, b"Access denied", &HashMap::new()));
}

#[test]
fn test_blocked_cf_header() {
    let mut headers = HashMap::new();
    headers.insert("cf-chl-bypass".to_string(), "1".to_string());
    assert!(is_blocked_response(200, b"normal content", &headers));
}

#[test]
fn test_normal_page_not_blocked() {
    let body = b"<html><body><h1>Hello World</h1><p>Content here</p></body></html>";
    assert!(!is_blocked_response(200, body, &HashMap::new()));
}

// === 闆嗘垚娴嬭瘯锛堟湰鍦版湇鍔″櫒锛?==

#[tokio::test]
async fn test_auto_mode_with_local_server() {
    use wisp::crawl::{SpiderBuilder, Engine};
    use wisp::crawl::auto::SelectorTracker;
    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // 妯℃嫙涓€涓?SPA 椤甸潰锛欻TML 涓?.product 涓虹┖锛堥渶瑕?JS 娓叉煋锛?
    let html = r#"<html><body><div id="app"></div><script>/* render products */</script></body></html>"#;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let html = html;
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(), html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });

    let base_url = format!("http://{}/products/1", addr);

    // 浣跨敤 Auto 妯″紡 + css() 杩借釜
    let spider = SpiderBuilder::new("auto-test")
        .start_urls(vec![base_url.clone()])
        .mode(FetchMode::Auto)
        .obey_robots(false)
        .on("default", |resp| async move {
            // 浣跨敤 resp.css() 瑙﹀彂杩借釜
            let products = resp.css(".product");
            let items: Vec<Value> = products.iter().map(|p| {
                serde_json::json!({ "text": p.text() })
            }).collect();
            (items, vec![])
        })
        .build();

    // Auto 妯″紡浼氭娴嬪埌 .product 杩斿洖 0 鑺傜偣
    // 浣嗙敱浜庢湰鍦版湇鍔″櫒涓嶆敮鎸?Dynamic锛堟棤 Chrome锛夛紝鍗囩骇浼氬け璐?
    // 杩欓噷涓昏楠岃瘉 Auto 閫昏緫涓?panic 涓旀甯稿畬鎴?
    let engine = Engine::infra()
        .max_pages(1)
        .build()
        .unwrap();
    let (stats, _items) = engine.run(spider).await.unwrap();

    // 椤甸潰搴旇琚埇鍙栵紙鍗充娇鍗囩骇澶辫触锛孒TTP 缁撴灉浠嶈浣跨敤锛?
    assert_eq!(stats.pages_crawled, 1);
}

#[tokio::test]
async fn test_auto_mode_static_page_no_upgrade() {
    use wisp::crawl::{SpiderBuilder, Engine};
    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // 闈欐€侀〉闈細.product 鏈夊唴瀹?
    let html = r#"<html><body><div class="product">Item 1</div><div class="product">Item 2</div></body></html>"#;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let html = html;
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(), html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });

    let base_url = format!("http://{}/products/1", addr);

    let spider = SpiderBuilder::new("auto-static")
        .start_urls(vec![base_url])
        .mode(FetchMode::Auto)
        .obey_robots(false)
        .on("default", |resp| async move {
            let products = resp.css(".product");
            let items: Vec<Value> = products.iter().map(|p| {
                serde_json::json!({ "text": p.text() })
            }).collect();
            (items, vec![])
        })
        .build();

    let engine = Engine::infra()
        .max_pages(1)
        .build()
        .unwrap();
    let (stats, _items) = engine.run(spider).await.unwrap();

    // 闈欐€侀〉闈細HTTP 鍗冲彲锛屼笉鍗囩骇锛? 涓?item
    assert_eq!(stats.pages_crawled, 1);
    assert_eq!(stats.items_scraped, 2);
}
