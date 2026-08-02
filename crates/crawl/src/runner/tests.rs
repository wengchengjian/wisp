use super::*;
use std::collections::HashSet;

use crate::{CrawlState, Request};

#[test]
fn engine_builder_transport_config() {
    let engine = Engine::infra()
        .headers(vec![("Accept".into(), "text/html".into())])
        .ua_rotation(crate::middleware::UaRotationMiddleware::desktop())
        .cookie_challenge(true)
        .build()
        .unwrap();
    assert_eq!(engine.config.headers.len(), 1);
    assert!(engine.runtime.ua_middleware.is_some());
    assert!(engine.config.cookie_challenge);
    assert!(
        !engine.config.dynamic_upgrade,
        "默认不开启 DynamicUpgrade 扫描"
    );
    assert!(
        Engine::infra()
            .dynamic_upgrade(true)
            .build()
            .unwrap()
            .config
            .dynamic_upgrade
    );
}

#[test]
fn merge_checkpoint_states_deduplicates_urls() {
    let req_a = Request::get("https://example.com/a");
    let req_b = Request::get("https://example.com/b");

    let mut s1 = CrawlState::new("spider-a".into());
    s1.pending_urls = vec![req_a.clone(), req_b.clone()];
    s1.seen_urls = HashSet::from([
        "https://example.com/a".into(),
        "https://example.com/b".into(),
    ]);

    let mut s2 = CrawlState::new("spider-b".into());
    s2.pending_urls = vec![req_a.clone()];
    s2.seen_urls = HashSet::from(["https://example.com/a".into()]);

    let (pending, seen) = super::setup::checkpoint::merge_checkpoint_states(vec![s1, s2]);
    assert_eq!(pending.len(), 2, "相同 URL 只应入队一次");
    assert_eq!(seen.len(), 2);
}

#[test]
fn engine_builder_accepts_existing_fetch_client() {
    use std::sync::Arc;
    use wisp_fetcher::{FetchClient, FetchClientConfig};

    let client = Arc::new(FetchClient::new(FetchClientConfig::default()).unwrap());
    let engine = Engine::infra()
        .fetch_client(Arc::clone(&client))
        .build()
        .unwrap();
    assert!(Arc::ptr_eq(&engine.runtime.fetch_client, &client));
    assert_eq!(
        engine.config.transport.max_concurrent_pages,
        client.config().max_concurrent_pages
    );
    assert_eq!(
        engine.config.transport.http.timeout,
        client.config().http.timeout
    );
}

#[test]
fn engine_builder_rejects_zero_max_concurrent() {
    let err = match Engine::infra().max_concurrent(0).build() {
        Ok(_) => panic!("max_concurrent=0 应构建失败"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("max_concurrent"),
        "错误应说明 max_concurrent: {err}"
    );
}

#[test]
fn checkpoint_restores_full_stats() {
    use std::collections::HashMap;

    let mut state = CrawlState::new("s".into());
    state.status_codes = HashMap::from([(200, 10)]);
    state.blocked = 5;
    state.retries = 6;
    state.offsite = 7;
    state.cache_hits = 8;

    let stats = crate::SpiderStats::new();
    stats.restore_from(&state);
    assert_eq!(stats.blocked.load(std::sync::atomic::Ordering::SeqCst), 5);
    assert_eq!(stats.retries.load(std::sync::atomic::Ordering::SeqCst), 6);
    assert_eq!(stats.offsite.load(std::sync::atomic::Ordering::SeqCst), 7);
    assert_eq!(
        stats.cache_hits.load(std::sync::atomic::Ordering::SeqCst),
        8
    );
    assert_eq!(stats.status_codes_snapshot(), HashMap::from([(200, 10)]));
}
