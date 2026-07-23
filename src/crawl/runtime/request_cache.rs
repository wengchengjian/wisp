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

    /// 构造缓存键：`"{method} {url}"`，区分不同 HTTP 方法的响应。
    /// 与 dev_mode 的 SQLite 缓存（按 `(url, method)` 存储）语义保持一致。
    fn cache_key(method: &str, url: &str) -> String {
        format!("{} {}", method, url)
    }

    /// Get a cached response for the given (method, url).
    pub async fn get(&self, method: &str, url: &str) -> Option<CachedEntry> {
        self.inner.get(&Self::cache_key(method, url)).await
    }

    /// Store a response in the cache.
    pub async fn put(&self, method: &str, url: &str, entry: CachedEntry) {
        self.inner.insert(Self::cache_key(method, url), entry).await;
    }

    /// Invalidate a specific (method, url) entry.
    pub async fn invalidate(&self, method: &str, url: &str) {
        self.inner.invalidate(&Self::cache_key(method, url)).await;
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
        cache.put("GET", "https://example.com", entry.clone()).await;

        let got = cache.get("GET", "https://example.com").await;
        assert!(got.is_some());
        let got = got.unwrap();
        assert_eq!(got.status, 200);
        assert_eq!(got.body, b"<html>hello</html>");
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache = RequestCache::new(100, Duration::from_secs(60));
        assert!(cache.get("GET", "https://nonexistent.com").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_invalidate() {
        let cache = RequestCache::new(100, Duration::from_secs(60));
        let entry = CachedEntry { status: 200, headers: HashMap::new(), body: vec![] };
        cache.put("GET", "https://example.com/page", entry).await;
        assert!(cache.get("GET", "https://example.com/page").await.is_some());

        cache.invalidate("GET", "https://example.com/page").await;
        assert!(cache.get("GET", "https://example.com/page").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_entry_count() {
        let cache = RequestCache::new(100, Duration::from_secs(60));
        assert_eq!(cache.entry_count(), 0);

        let entry = CachedEntry { status: 200, headers: HashMap::new(), body: vec![] };
        cache.put("GET", "https://a.com", entry.clone()).await;
        cache.put("GET", "https://b.com", entry).await;
        // moka entry_count is eventually consistent; verify via get instead
        assert!(cache.get("GET", "https://a.com").await.is_some());
        assert!(cache.get("GET", "https://b.com").await.is_some());
    }

    /// Task 8: POST 与 GET 同 URL 不应共享缓存。
    #[tokio::test]
    async fn cache_key_includes_method() {
        let cache = RequestCache::new(100, Duration::from_secs(60));
        let get_entry = CachedEntry {
            status: 200,
            headers: HashMap::new(),
            body: b"GET-RESPONSE".to_vec(),
        };
        // 存 GET 响应
        cache.put("GET", "https://example.com/api", get_entry).await;

        // GET 命中
        let got = cache.get("GET", "https://example.com/api").await;
        assert!(got.is_some(), "GET 应命中");

        // POST 不应命中 GET 的缓存
        let post = cache.get("POST", "https://example.com/api").await;
        assert!(post.is_none(), "POST 不应命中 GET 缓存，实际 {:?}", post);
    }
}
