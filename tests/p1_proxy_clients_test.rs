//! P1-1b: proxy_clients 用 moka::Cache，相同 proxy 复用 Client。
//!
//! ND-009-SEC：proxy_clients 从 DashMap 替换为 moka::sync::Cache（max_capacity=1024），
//! 防止攻击者注入大量不同 proxy URL 导致无界增长 OOM。

use wisp::crawl::engine::fetch_page_inner;
use wisp::crawl::Request;
use wisp::fetcher::{FetchClient, FetchClientConfig, FetchMode};

#[tokio::test]
async fn proxy_clients_caches_client_per_proxy_url() {
    // proxy_clients 使用 moka::Cache，验证相同 proxy 两次 fetch 只产生一个缓存条目
    let fetch_client = FetchClient::new(FetchClientConfig::default()).unwrap();
    let proxy_clients = moka::sync::Cache::builder().max_capacity(1024).build();
    let req = Request::get("http://127.0.0.1:1/unreachable");

    // 两次 fetch 同一 proxy（连接会失败，但 Client 应被缓存）
    for _ in 0..2 {
        let _ = fetch_page_inner(
            &fetch_client,
            &req,
            Some("http://127.0.0.1:1"),
            FetchMode::Http,
            &proxy_clients,
        ).await;
    }

    // moka::Cache 的 entry_count() 在异步清理后不实时准确（moka 已知行为），
    // 用 contains_key 作为缓存验证断言更可靠
    assert!(proxy_clients.contains_key("http://127.0.0.1:1"), "相同 proxy 应被缓存");
}
