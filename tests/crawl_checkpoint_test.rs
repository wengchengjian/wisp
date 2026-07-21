//! Verify checkpoint save/load round-trip.

use wisp::crawl::{CrawlState, SpiderRequest, CrawlStats};
use wisp::storage::Store;

#[test]
fn test_checkpoint_save_load_roundtrip() {
    let store = Store::open_in_memory().unwrap();

    let stats = CrawlStats {
        items_scraped: 100,
        pages_crawled: 42,
        errors: 3,
        duration: std::time::Duration::from_millis(5678),
        ..Default::default()
    };
    let pending = vec![SpiderRequest::get("https://example.com/pending")];
    let state = CrawlState::from_stats("test-spider".to_string(), &stats, pending);

    let blob = bincode::serialize(&state).unwrap();
    store.save_checkpoint("test-spider", &blob, state.saved_at.timestamp()).unwrap();

    let loaded = store.load_checkpoint("test-spider").unwrap().expect("should be saved");
    let restored: CrawlState = bincode::deserialize(&loaded).unwrap();

    assert_eq!(restored.spider_name, "test-spider");
    assert_eq!(restored.pages_crawled, 42);
    assert_eq!(restored.items_scraped, 100);
    assert_eq!(restored.errors, 3);
    assert_eq!(restored.duration_ms, 5678);
    assert_eq!(restored.pending_urls.len(), 1);
    assert_eq!(restored.pending_urls[0].url, "https://example.com/pending");

    // 验证 to_stats 往返
    let restored_stats = restored.to_stats();
    assert_eq!(restored_stats.pages_crawled, 42);
    assert_eq!(restored_stats.duration, std::time::Duration::from_millis(5678));
}

#[test]
fn test_checkpoint_delete() {
    let store = Store::open_in_memory().unwrap();
    let state = CrawlState::new("s2".to_string());
    let blob = bincode::serialize(&state).unwrap();
    store.save_checkpoint("s2", &blob, 0).unwrap();
    assert!(store.load_checkpoint("s2").unwrap().is_some());

    store.delete_checkpoint("s2").unwrap();
    assert!(store.load_checkpoint("s2").unwrap().is_none());
}

#[test]
fn test_checkpoint_load_missing_returns_none() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.load_checkpoint("nonexistent").unwrap().is_none());
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
}
