//! Simple file-based response cache.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// File-based cache for HTTP responses.
pub struct ResponseCache {
    dir: PathBuf,
}

impl ResponseCache {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    /// Get cached response body for a URL.
    pub fn get(&self, url: &str) -> Option<Vec<u8>> {
        let path = self.path_for(url);
        std::fs::read(path).ok()
    }

    /// Store response body for a URL.
    pub fn put(&self, url: &str, body: &[u8]) {
        let path = self.path_for(url);
        let _ = std::fs::write(path, body);
    }

    /// Check if URL is cached.
    pub fn contains(&self, url: &str) -> bool {
        self.path_for(url).exists()
    }

    fn path_for(&self, url: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        let hash = hasher.finish();
        self.dir.join(format!("{:016x}.cache", hash))
    }
}
