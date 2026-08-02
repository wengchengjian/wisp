use super::*;
use std::sync::Arc;
use wisp_fetcher::{FetchClient, FetchClientConfig};

#[test]
fn test_robots_rules_default() {
    let r = RobotsRules::default();
    assert!(r.disallowed.is_empty());
    assert!(r.crawl_delay.is_none());
    assert!(r.request_rate.is_none());
}

#[test]
fn test_parse_crawl_delay() {
    let text = "User-agent: *\nCrawl-delay: 5\nDisallow: /private";
    let rules = parse_robots_text(text);
    assert_eq!(rules.crawl_delay, Some(5.0));
    assert!(rules.disallowed.contains(&"/private".to_string()));
    assert!(rules.request_rate.is_none());
}

#[test]
fn test_parse_crawl_delay_ignored_outside_wildcard_section() {
    // Crawl-delay 在非 * 的 User-agent 段内不应被采集
    let text = "User-agent: GoogleBot\nCrawl-delay: 10\n\nUser-agent: *\nDisallow: /admin";
    let rules = parse_robots_text(text);
    assert_eq!(rules.crawl_delay, None);
    assert!(rules.disallowed.contains(&"/admin".to_string()));
}

#[test]
fn test_parse_request_rate() {
    // Request-rate: 1/5 → 0.2 requests per second
    let text = "User-agent: *\nRequest-rate: 1/5\nDisallow: /slow";
    let rules = parse_robots_text(text);
    assert_eq!(rules.request_rate, Some(0.2));
    assert!(rules.disallowed.contains(&"/slow".to_string()));
}

#[test]
fn test_parse_request_rate_with_optional_comment() {
    // Request-rate: 2/10 (during peak) → 0.2 req/s
    let text = "User-agent: *\nRequest-rate: 2/10 (during peak)";
    let rules = parse_robots_text(text);
    assert_eq!(rules.request_rate, Some(0.2));
}

#[test]
fn test_parse_ignores_comments_and_empty_lines() {
    let text = "# 注释行\n\nUser-agent: *\n# 中间注释\nDisallow: /secret\n";
    let rules = parse_robots_text(text);
    assert!(rules.disallowed.contains(&"/secret".to_string()));
}

#[test]
fn test_parse_empty_disallow_ignored() {
    // Disallow: (空) 按 RFC 9309 表示允许全部，不应加入 disallowed
    let text = "User-agent: *\nDisallow:";
    let rules = parse_robots_text(text);
    assert!(rules.disallowed.is_empty());
}

#[test]
fn test_parse_invalid_crawl_delay_ignored() {
    let text = "User-agent: *\nCrawl-delay: not-a-number";
    let rules = parse_robots_text(text);
    assert!(rules.crawl_delay.is_none());
}

#[test]
fn test_parse_invalid_request_rate_ignored() {
    let text = "User-agent: *\nRequest-rate: abc";
    let rules = parse_robots_text(text);
    assert!(rules.request_rate.is_none());
}

#[test]
fn test_parse_empty_text_returns_default() {
    let rules = parse_robots_text("");
    assert!(rules.disallowed.is_empty());
    assert!(rules.crawl_delay.is_none());
    assert!(rules.request_rate.is_none());
}

#[test]
fn test_is_empty_rules() {
    // 默认规则视为空（fetch 失败的返回值）
    assert!(RobotsRules::default().is_empty_rules());

    // 仅有 disallowed 不算空
    let mut r = RobotsRules::default();
    r.disallowed.push("/x".to_string());
    assert!(!r.is_empty_rules());

    // 仅有 crawl_delay 不算空
    let mut r = RobotsRules::default();
    r.crawl_delay = Some(1.0);
    assert!(!r.is_empty_rules());

    // 仅有 request_rate 不算空
    let mut r = RobotsRules::default();
    r.request_rate = Some(0.5);
    assert!(!r.is_empty_rules());
}

#[test]
fn test_domain_key_preserves_port() {
    // 验证 url::Url::port() 在非默认端口下返回 Some，
    // 用以锁定 rules_for 构造 domain key 时包含端口的依赖假设。
    let parsed = url::Url::parse("http://example.com:8080/x").unwrap();
    assert_eq!(parsed.port(), Some(8080));
    let parsed_default = url::Url::parse("http://example.com/x").unwrap();
    assert_eq!(parsed_default.port(), None);
}

#[tokio::test]
async fn test_negative_cache_on_fetch_failure() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::net::TcpListener;

    // 起一个 server，accept 后立即 drop socket（模拟连接重置，fetch 失败）
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let fetch_count = Arc::new(AtomicUsize::new(0));
    let fetch_count_clone = fetch_count.clone();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            fetch_count_clone.fetch_add(1, Ordering::SeqCst);
            drop(socket); // 模拟连接重置
        }
    });

    let client = FetchClient::new(FetchClientConfig {
        max_concurrent_pages: 0,
        ..Default::default()
    })
    .unwrap();
    // TTL 100ms，测试不用等太久
    let cache = RobotsCache::with_negative_ttl(Duration::from_millis(100));
    let url = format!("http://{}/page", addr);

    // 第一次：fetch 失败 → 缓存 Failed
    let rules1 = cache.rules_for(&client, &url).await;
    assert!(rules1.is_empty_rules());
    assert_eq!(fetch_count.load(Ordering::SeqCst), 1, "第一次应触发 fetch");

    // 第二次：TTL 内 → 不重试，直接返回 default
    let rules2 = cache.rules_for(&client, &url).await;
    assert!(rules2.is_empty_rules());
    assert_eq!(
        fetch_count.load(Ordering::SeqCst),
        1,
        "TTL 内不应重试 fetch"
    );

    // 等 TTL 过期
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 第三次：TTL 过期 → 重新 fetch
    let rules3 = cache.rules_for(&client, &url).await;
    assert!(rules3.is_empty_rules());
    assert_eq!(
        fetch_count.load(Ordering::SeqCst),
        2,
        "TTL 过期后应重新 fetch"
    );
}

#[tokio::test]
async fn test_successful_fetch_caches_rules() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // 起一个 server，返回有效 robots.txt
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let fetch_count = Arc::new(AtomicUsize::new(0));
    let fetch_count_clone = fetch_count.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            fetch_count_clone.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let body = "User-agent: *\nDisallow: /private\n";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });

    let client = FetchClient::new(FetchClientConfig {
        max_concurrent_pages: 0,
        ..Default::default()
    })
    .unwrap();
    let cache = RobotsCache::with_negative_ttl(Duration::from_secs(60));
    let url = format!("http://{}/page", addr);

    // 第一次：fetch 成功 → 缓存 Rules
    let rules1 = cache.rules_for(&client, &url).await;
    assert!(rules1.disallowed.contains(&"/private".to_string()));
    assert_eq!(fetch_count.load(Ordering::SeqCst), 1, "第一次应触发 fetch");

    // 第二次：缓存命中 → 不重新 fetch
    let rules2 = cache.rules_for(&client, &url).await;
    assert!(rules2.disallowed.contains(&"/private".to_string()));
    assert_eq!(
        fetch_count.load(Ordering::SeqCst),
        1,
        "缓存命中不应重新 fetch"
    );
}
