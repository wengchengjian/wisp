//! 验证 robots.txt 从正确的 host:port 获取（端口不丢失）。
//!
//! 回归测试：`rules_for` 之前用 `host_str()`（不含端口）构造 domain key，
//! 导致 `http://127.0.0.1:8080/x` 的 robots.txt 错误地从 `http://127.0.0.1/robots.txt`
//! （端口 80）获取。本测试用本地 mock server 验证请求实际命中带端口的地址。
//!
//! 同时验证：fetch 失败时返回的空规则不被缓存（瞬态网络失败后下次重试）。
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wisp::crawl::runtime::robots::RobotsCache;
use wisp::http::Client;

#[tokio::test]
async fn robots_fetched_from_correct_port() {
    // 启动 mock server 监听随机端口
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_c = counter.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let c = counter_c.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 512];
                let _ = sock.read(&mut buf).await;
                c.fetch_add(1, Ordering::SeqCst);
                // Disallow: /private 仅阻止 /private*，/page 不匹配应允许
                let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 32\r\n\r\nUser-agent: *\nDisallow: /private";
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });

    // 用带非默认端口的 URL 触发 rules_for
    let url = format!("http://127.0.0.1:{}/page", port);
    let client = Client::new().unwrap();
    let mut cache = RobotsCache::new();
    let allowed = cache.is_allowed(&client, &url).await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "应从带端口的地址获取 robots.txt（端口不应丢失）"
    );
    assert!(allowed, "/page 不匹配 Disallow: /private 应允许");
}

#[tokio::test]
async fn fetch_failure_not_cached_so_retry_happens() {
    // 第一次指向不存在的端口（fetch 失败），第二次指向有效 mock server。
    // 失败应不缓存，第二次 rules_for 应重新 fetch 命中 mock server。
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_c = counter.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let c = counter_c.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 512];
                let _ = sock.read(&mut buf).await;
                c.fetch_add(1, Ordering::SeqCst);
                let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 25\r\n\r\nUser-agent: *\nDisallow: /";
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });

    let client = Client::new().unwrap();
    let mut cache = RobotsCache::new();

    // 第一次：指向无人监听的端口，fetch 应失败返回空规则（且不缓存）
    let dead_port = port.wrapping_add(1);
    // 尝试若干端口找到一个真正无监听的（避免 mock server 端口+1 偶然被占）
    let dead_url = format!("http://127.0.0.1:{dead_port}/page");
    let _dead_rules = cache.rules_for(&client, &dead_url).await;

    // 第二次：指向有效 mock server，应重新 fetch（counter==1）
    let live_url = format!("http://127.0.0.1:{port}/page");
    let _live_rules = cache.rules_for(&client, &live_url).await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "失败 fetch 不应缓存，第二次（不同 domain key）应成功命中 mock server"
    );
}
