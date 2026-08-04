//! Verify checkpoint save/load round-trip.

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use wisp::crawl::{CrawlRequest, CrawlState};
use wisp::storage::MemoryStore;

#[tokio::test]
async fn test_checkpoint_save_load_roundtrip() {
    let store = MemoryStore::default();

    let mut state = CrawlState::new("test-spider".to_string());
    state.pending_urls = vec![CrawlRequest::get("https://example.com/pending")];
    state.stats.pages_crawled = 42;
    state.stats.items_scraped = 100;
    state.stats.errors = 3;
    state.stats.duration = Duration::from_millis(5678);
    state.stats.status_code_counts = HashMap::from([(200, 40), (404, 2)]);
    state.stats.blocked_requests = 1;
    state.stats.retry_count = 2;
    state.stats.offsite_requests_count = 3;
    state.stats.cache_hits = 4;
    state.stats.callback_pages = HashMap::from([("detail".to_string(), 2)]);

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
    assert_eq!(restored.stats.pages_crawled, 42);
    assert_eq!(restored.stats.items_scraped, 100);
    assert_eq!(restored.stats.errors, 3);
    assert_eq!(restored.stats.duration, Duration::from_millis(5678));
    assert_eq!(restored.pending_urls.len(), 1);
    assert_eq!(restored.pending_urls[0].url, "https://example.com/pending");
    assert_eq!(
        restored.stats.status_code_counts,
        HashMap::from([(200, 40), (404, 2)])
    );
    assert_eq!(restored.stats.blocked_requests, 1);
    assert_eq!(restored.stats.retry_count, 2);
    assert_eq!(restored.stats.offsite_requests_count, 3);
    assert_eq!(restored.stats.cache_hits, 4);
    assert_eq!(restored.stats.callback_pages.get("detail"), Some(&2));

    let restored_stats = restored.to_stats();
    assert_eq!(restored_stats.pages_crawled, 42);
    assert_eq!(restored_stats.duration, Duration::from_millis(5678));
    assert_eq!(
        restored_stats.status_code_counts,
        HashMap::from([(200, 40), (404, 2)])
    );
    assert_eq!(restored_stats.blocked_requests, 1);
    assert_eq!(restored_stats.retry_count, 2);
    assert_eq!(restored_stats.offsite_requests_count, 3);
    assert_eq!(restored_stats.cache_hits, 4);
    assert_eq!(restored_stats.callback_pages.get("detail"), Some(&2));
}

#[tokio::test]
async fn test_checkpoint_delete() {
    let store = MemoryStore::default();
    let state = CrawlState::new("s2".to_string());
    let blob = bincode::serialize(&state).unwrap();
    wisp::storage::save_checkpoint(&store, "s2", &blob)
        .await
        .unwrap();
    assert!(
        wisp::storage::load_checkpoint(&store, "s2")
            .await
            .unwrap()
            .is_some()
    );

    wisp::storage::delete_checkpoint(&store, "s2")
        .await
        .unwrap();
    assert!(
        wisp::storage::load_checkpoint(&store, "s2")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_checkpoint_load_missing_returns_none() {
    let store = MemoryStore::default();
    assert!(
        wisp::storage::load_checkpoint(&store, "nonexistent")
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_crawl_state_new_defaults() {
    let state = CrawlState::new("fresh".to_string());
    assert_eq!(state.spider_name, "fresh");
    assert_eq!(state.stats.pages_crawled, 0);
    assert_eq!(state.stats.items_scraped, 0);
    assert_eq!(state.stats.errors, 0);
    assert_eq!(state.stats.duration, Duration::ZERO);
    assert!(state.pending_urls.is_empty());
    assert!(state.seen_urls.is_empty());
    assert!(state.stats.status_code_counts.is_empty());
    assert_eq!(state.stats.blocked_requests, 0);
    assert_eq!(state.stats.retry_count, 0);
    assert_eq!(state.stats.offsite_requests_count, 0);
    assert_eq!(state.stats.cache_hits, 0);
}

/// 验证 seen_urls 序列化层往返。
#[tokio::test]
async fn checkpoint_restore_preserves_seen_urls() {
    let store = MemoryStore::default();
    let mut state = CrawlState::new("test_spider".into());
    state.pending_urls = vec![CrawlRequest::get("https://example.com/pending")];
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
