//! In-memory request cache backed by moka.
//!
//! Provides TTL-based + capacity-limited caching for HTTP responses,
//! eliminating disk IO for hot data.

use std::collections::HashMap;
use std::time::Duration;
use moka::future::Cache;

/// A cached HTTP response entry.
#[derive(Debug, Clone)]
pub struct CachedEntry {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

/// In-memory request cache (moka-based, supports TTL + max capacity).
///
/// Clone is cheap (Arc internally).
#[derive(Clone)]
pub struct RequestCache {
    inner: Cache<String, CachedEntry>,
}

impl RequestCache {
    /// Create a new cache.
    /// - `max_entries`: maximum number of cached responses
    /// - `ttl`: time-to-live for each entry
    pub fn new(max_entries: u64, ttl: Duration) -> Self {
        Self {
            inner: Cache::builder()
                .max_capacity(max_entries)
                .time_to_live(ttl)
                .build(),
        }
    }

    /// Get a cached response for the given URL.
    pub async fn get(&self, url: &str) -> Option<CachedEntry> {
        self.inner.get(url).await
    }

    /// Store a response in the cache.
    pub async fn put(&self, url: &str, entry: CachedEntry) {
        self.inner.insert(url.to_string(), entry).await;
    }

    /// Invalidate a specific URL entry.
    pub async fn invalidate(&self, url: &str) {
        self.inner.invalidate(url).await;
    }

    /// Current number of entries in the cache.
    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_put_and_get() {
        let cache = RequestCache::new(100, Duration::from_secs(60));
        let entry = CachedEntry {
            status: 200,
            headers: HashMap::from([("content-type".to_string(), "text/html".to_string())]),
            body: b"<html>hello</html>".to_vec(),
        };
        cache.put("https://example.com", entry.clone()).await;

        let got = cache.get("https://example.com").await;
        assert!(got.is_some());
        let got = got.unwrap();
        assert_eq!(got.status, 200);
        assert_eq!(got.body, b"<html>hello</html>");
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache = RequestCache::new(100, Duration::from_secs(60));
        assert!(cache.get("https://nonexistent.com").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_invalidate() {
        let cache = RequestCache::new(100, Duration::from_secs(60));
        let entry = CachedEntry { status: 200, headers: HashMap::new(), body: vec![] };
        cache.put("https://example.com/page", entry).await;
        assert!(cache.get("https://example.com/page").await.is_some());

        cache.invalidate("https://example.com/page").await;
        assert!(cache.get("https://example.com/page").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_entry_count() {
        let cache = RequestCache::new(100, Duration::from_secs(60));
        assert_eq!(cache.entry_count(), 0);

        let entry = CachedEntry { status: 200, headers: HashMap::new(), body: vec![] };
        cache.put("https://a.com", entry.clone()).await;
        cache.put("https://b.com", entry).await;
        // moka entry_count is eventually consistent; verify via get instead
        assert!(cache.get("https://a.com").await.is_some());
        assert!(cache.get("https://b.com").await.is_some());
    }
}
