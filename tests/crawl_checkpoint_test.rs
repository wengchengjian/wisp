//! Verify checkpoint save/load round-trip.

use std::collections::{HashMap, HashSet};
use wisp::crawl::{CrawlState, Request};
use wisp::storage::MemoryStore;

#[tokio::test]
async fn test_checkpoint_save_load_roundtrip() {
    let store = MemoryStore::default();

    let mut state = CrawlState::new("test-spider".to_string());
    state.pending_urls = vec![Request::get("https://example.com/pending")];
    state.pages_crawled = 42;
    state.items_scraped = 100;
    state.errors = 3;
    state.duration_ms = 5678;
    state.status_codes = HashMap::from([(200, 40), (404, 2)]);
    state.blocked = 1;
    state.retries = 2;
    state.offsite = 3;
    state.cache_hits = 4;

    let blob = bincode::serialize(&state).unwrap();
    wisp::storage::save_checkpoint(&store, "test-spider", &blob)
        .await
        .unwrap();

    let loaded = wisp::storage::load_checkpoint(&store, "test-spider")
        .await
        .unwrap()
        .expect("should be saved");
    let restored: CrawlState = bincode::deserialize(&loaded).unwrap();

    assert_eq!(restored.spider_name, "test-spider");
    assert_eq!(restored.pages_crawled, 42);
    assert_eq!(restored.items_scraped, 100);
    assert_eq!(restored.errors, 3);
    assert_eq!(restored.duration_ms, 5678);
    assert_eq!(restored.pending_urls.len(), 1);
    assert_eq!(restored.pending_urls[0].url, "https://example.com/pending");
    assert_eq!(restored.status_codes, HashMap::from([(200, 40), (404, 2)]));
    assert_eq!(restored.blocked, 1);
    assert_eq!(restored.retries, 2);
    assert_eq!(restored.offsite, 3);
    assert_eq!(restored.cache_hits, 4);

    let restored_stats = restored.to_stats();
    assert_eq!(restored_stats.pages_crawled, 42);
    assert_eq!(
        restored_stats.duration,
        std::time::Duration::from_millis(5678)
    );
    assert_eq!(
        restored_stats.status_code_counts,
        HashMap::from([(200, 40), (404, 2)])
    );
    assert_eq!(restored_stats.blocked_requests, 1);
    assert_eq!(restored_stats.retry_count, 2);
    assert_eq!(restored_stats.offsite_requests_count, 3);
    assert_eq!(restored_stats.cache_hits, 4);
}

#[tokio::test]
async fn test_checkpoint_delete() {
    let store = MemoryStore::default();
    let state = CrawlState::new("s2".to_string());
    let blob = bincode::serialize(&state).unwrap();
    wisp::storage::save_checkpoint(&store, "s2", &blob)
        .await
        .unwrap();
    assert!(wisp::storage::load_checkpoint(&store, "s2")
        .await
        .unwrap()
        .is_some());

    wisp::storage::delete_checkpoint(&store, "s2")
        .await
        .unwrap();
    assert!(wisp::storage::load_checkpoint(&store, "s2")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_checkpoint_load_missing_returns_none() {
    let store = MemoryStore::default();
    assert!(wisp::storage::load_checkpoint(&store, "nonexistent")
        .await
        .unwrap()
        .is_none());
}

#[test]
fn test_crawl_state_new_defaults() {
    let state = CrawlState::new("fresh".to_string());
    assert_eq!(state.spider_name, "fresh");
    assert_eq!(state.pages_crawled, 0);
    assert_eq!(state.items_scraped, 0);
    assert_eq!(state.errors, 0);
    assert_eq!(state.duration_ms, 0);
    assert!(state.pending_urls.is_empty());
    assert!(state.seen_urls.is_empty());
    assert!(state.status_codes.is_empty());
    assert_eq!(state.blocked, 0);
    assert_eq!(state.retries, 0);
    assert_eq!(state.offsite, 0);
    assert_eq!(state.cache_hits, 0);
}

/// 验证 seen_urls 序列化层往返。
#[tokio::test]
async fn checkpoint_restore_preserves_seen_urls() {
    let store = MemoryStore::default();
    let mut state = CrawlState::new("test_spider".into());
    state.pending_urls = vec![Request::get("https://example.com/pending")];
    state.seen_urls = HashSet::from(["https://example.com/already-crawled".to_string()]);
    let blob = bincode::serialize(&state).unwrap();
    wisp::storage::save_checkpoint(&store, "test_spider", &blob)
        .await
        .unwrap();

    let loaded = wisp::storage::load_checkpoint(&store, "test_spider")
        .await
        .unwrap()
        .unwrap();
    let restored: CrawlState = bincode::deserialize(&loaded).unwrap();
    assert!(
        restored
            .seen_urls
            .contains("https://example.com/already-crawled"),
        "seen_urls 必须被持久化与恢复"
    );
}
