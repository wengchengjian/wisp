//! P1-1b: proxy_clients 用 DashMap，相同 proxy 复用 Client。

use std::sync::Arc;
use wisp::crawl::engine::fetch_page_inner;
use wisp::crawl::SpiderRequest;
use wisp::fetcher::FetchMode;
use wisp::http::{Client, Config};

#[tokio::test]
async fn proxy_clients_caches_client_per_proxy_url() {
    // proxy_clients 暴露为 DashMap，验证相同 proxy 两次 fetch 只产生一个缓存条目
    let client = Arc::new(Client::builder().build().unwrap());
    let config = Config::default();
    let proxy_clients = Arc::new(dashmap::DashMap::new());
    let req = SpiderRequest::get("http://127.0.0.1:1/unreachable");

    // 两次 fetch 同一 proxy（连接会失败，但 Client 应被缓存）
    for _ in 0..2 {
        let _ = fetch_page_inner(
            &client,
            &req,
            Some("http://127.0.0.1:1"),
            FetchMode::Http,
            &config,
            &proxy_clients,
        ).await;
    }

    assert_eq!(proxy_clients.len(), 1, "相同 proxy 应只缓存 1 个 Client");
    assert!(proxy_clients.contains_key("http://127.0.0.1:1"));
}
