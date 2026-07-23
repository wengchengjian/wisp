//! Verify checkpoint save/load round-trip.

use wisp::crawl::{CrawlState, SpiderRequest, CrawlStats};
use wisp::storage::Store;
use std::collections::HashSet;

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

/// Task 3：验证 CrawlState 序列化层 seen_urls 往返。
///
/// 此测试模拟 save_checkpoint 写入：手动构造含 seen_urls 的 CrawlState，
/// 经 bincode 序列化 → Store 持久化 → 加载 → 反序列化，确认 seen_urls 不丢失。
/// 注意：此处只验证序列化层契约；save_checkpoint 真正写入 seen_urls 的行为
/// 由 engine.rs 内部 lib 测试 `save_checkpoint_persists_seen_urls` 覆盖。
#[tokio::test]
async fn checkpoint_restore_preserves_seen_urls() {
    let store = Store::open_in_memory().unwrap();
    // 模拟 save_checkpoint 写入：构造含 seen_urls 的 CrawlState
    let mut state = CrawlState::new("test_spider".into());
    state.pending_urls = vec![SpiderRequest::get("https://example.com/pending")];
    state.seen_urls = HashSet::from([
        "https://example.com/already-crawled".to_string(),
    ]);
    let blob = bincode::serialize(&state).unwrap();
    store.save_checkpoint("test_spider", &blob, 0).unwrap();

    // 加载并验证 seen_urls 被持久化
    let loaded = store.load_checkpoint("test_spider").unwrap().unwrap();
    let restored: CrawlState = bincode::deserialize(&loaded).unwrap();
    assert!(restored.seen_urls.contains("https://example.com/already-crawled"),
        "seen_urls 必须被持久化与恢复");
}
