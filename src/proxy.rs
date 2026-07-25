//! Proxy pool management with rotation strategies.

use std::sync::atomic::{AtomicUsize, Ordering};

/// How to pick the next proxy from the pool.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RotationStrategy {
    /// Use proxies in order, cycling back to the start.
    Sequential,
    /// Pick a random proxy each time.
    Random,
    /// Stick with one proxy per session (index set at creation).
    Sticky,
}

/// Manages a pool of proxy URLs and rotates through them.
pub struct ProxyPool {
    proxies: Vec<String>,
    strategy: RotationStrategy,
    index: AtomicUsize,
}

impl ProxyPool {
    /// Create a new proxy pool.
    ///
    /// Proxies should be in format: `http://user:pass@host:port` or `http://host:port`
    ///
    /// Sticky 模式初始索引随机化，避免所有实例都固定到第一个代理。
    pub fn new(proxies: Vec<String>, strategy: RotationStrategy) -> Self {
        use rand::rngs::{SmallRng, SysRng};
        use rand::{RngExt, SeedableRng};
        let initial = if strategy == RotationStrategy::Sticky && !proxies.is_empty() {
            SmallRng::try_from_rng(&mut SysRng).expect("OS RNG failed").random_range(0..proxies.len())
        } else {
            0
        };
        Self {
            proxies,
            strategy,
            index: AtomicUsize::new(initial),
        }
    }

    /// Get the next proxy according to the rotation strategy.
    pub fn next(&self) -> Option<String> {
        if self.proxies.is_empty() {
            return None;
        }

        let idx = match self.strategy {
            RotationStrategy::Sequential => {
                let len = self.proxies.len();
                let i = self.index.fetch_add(1, Ordering::Relaxed).wrapping_rem(len);
                i
            }
            RotationStrategy::Random => {
                use rand::rngs::{SmallRng, SysRng};
                use rand::{RngExt, SeedableRng};
                SmallRng::try_from_rng(&mut SysRng).expect("OS RNG failed").random_range(0..self.proxies.len())
            }
            RotationStrategy::Sticky => {
                self.index.load(Ordering::Relaxed) % self.proxies.len()
            }
        };

        Some(self.proxies[idx].clone())
    }

    /// Number of proxies in the pool.
    pub fn len(&self) -> usize {
        self.proxies.len()
    }

    /// 代理池是否为空。
    pub fn is_empty(&self) -> bool {
        self.proxies.is_empty()
    }

    /// Format a proxy URL as a Chrome `--proxy-server` argument value.
    pub fn to_chrome_arg(proxy: &str) -> String {
        proxy.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequential_rotation() {
        let pool = ProxyPool::new(
            vec!["http://p1:8080".into(), "http://p2:8080".into(), "http://p3:8080".into()],
            RotationStrategy::Sequential,
        );
        assert_eq!(pool.next().unwrap(), "http://p1:8080");
        assert_eq!(pool.next().unwrap(), "http://p2:8080");
        assert_eq!(pool.next().unwrap(), "http://p3:8080");
        assert_eq!(pool.next().unwrap(), "http://p1:8080"); // cycles
    }

    #[test]
    fn test_empty_pool() {
        let pool = ProxyPool::new(vec![], RotationStrategy::Sequential);
        assert!(pool.next().is_none());
    }

    #[test]
    fn test_sticky() {
        let pool = ProxyPool::new(
            vec!["http://p1:8080".into(), "http://p2:8080".into()],
            RotationStrategy::Sticky,
        );
        let first = pool.next().unwrap();
        assert_eq!(pool.next().unwrap(), first);
        assert_eq!(pool.next().unwrap(), first);
    }
}
