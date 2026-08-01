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
    assert_eq!(engine.headers.len(), 1);
    assert!(engine.ua_middleware.is_some());
    assert!(engine.cookie_challenge);
    assert!(!engine.dynamic_upgrade, "默认不开启 DynamicUpgrade 扫描");
    assert!(
        Engine::infra()
            .dynamic_upgrade(true)
            .build()
            .unwrap()
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
    assert!(Arc::ptr_eq(&engine.fetch_client, &client));
}
