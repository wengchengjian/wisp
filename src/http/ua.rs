//! Random User-Agent rotation.
//!
//! Provides a pool of real browser UA strings for per-request rotation,
//! reducing fingerprint detection risk.

use rand::seq::IndexedRandom;

/// Real desktop User-Agent strings (Chrome/Edge 136, matching default TLS fingerprint Profile::Chrome136).
const DESKTOP_UAS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.7103.25 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.7103.25 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.7103.25 Safari/537.36",
];

/// Real mobile User-Agent strings (iOS Safari / Android Chrome).
const MOBILE_UAS: &[&str] = &[
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (Linux; Android 14; SM-S928B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36",
    "Mozilla/5.0 (iPad; CPU OS 18_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (Linux; Android 15; SM-A556B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/131.0.6778.73 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (Linux; Android 14; Pixel 8 Pro) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
];

/// UA rotator - picks a random UA from the pool on each call.
///
/// 内置池使用 `&'static str` 避免堆分配（D-016）。
pub struct UaRotator {
    pool: UaPool,
}

enum UaPool {
    Static(&'static [&'static str]),
    Owned(Vec<String>),
}

impl UaRotator {
    /// Desktop browser UA pool.
    #[must_use]
    pub fn desktop() -> Self {
        Self {
            pool: UaPool::Static(DESKTOP_UAS),
        }
    }

    /// Mobile browser UA pool.
    #[must_use]
    pub fn mobile() -> Self {
        Self {
            pool: UaPool::Static(MOBILE_UAS),
        }
    }

    /// Custom UA pool.
    #[must_use]
    pub fn custom(uas: Vec<String>) -> Self {
        Self {
            pool: UaPool::Owned(uas),
        }
    }

    /// Pick a random UA from the pool.
    pub fn next(&self) -> &str {
        use rand::rngs::{SmallRng, SysRng};
        use rand::SeedableRng;
        let mut rng = SmallRng::try_from_rng(&mut SysRng).expect("OS RNG failed");
        match &self.pool {
            UaPool::Static(pool) => pool.choose(&mut rng).copied().unwrap_or(""),
            UaPool::Owned(pool) => pool
                .choose(&mut rng)
                .map_or("", std::string::String::as_str),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_desktop_rotator_returns_valid_ua() {
        let rotator = UaRotator::desktop();
        let ua = rotator.next();
        assert!(
            ua.contains("Mozilla/5.0"),
            "UA should start with Mozilla/5.0: {ua}"
        );
    }

    #[test]
    fn test_mobile_rotator_returns_valid_ua() {
        let rotator = UaRotator::mobile();
        let ua = rotator.next();
        assert!(
            ua.contains("Mozilla/5.0"),
            "UA should start with Mozilla/5.0: {ua}"
        );
    }

    #[test]
    fn test_custom_rotator() {
        let rotator = UaRotator::custom(vec!["MyBot/1.0".to_string()]);
        assert_eq!(rotator.next(), "MyBot/1.0");
    }

    #[test]
    fn test_rotator_produces_variety() {
        let rotator = UaRotator::desktop();
        let uas: std::collections::HashSet<&str> = (0..50).map(|_| rotator.next()).collect();
        assert!(
            uas.len() > 1,
            "Should produce multiple different UAs over 50 calls"
        );
    }
}
