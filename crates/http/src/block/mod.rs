//! Domain and ad blocking for bandwidth savings and privacy.
//!
//! 类似 Scrapling 的 block_domains + 内置广告域列表。
//! 在浏览器模式下拦截对指定域名（及其子域名）的请求，
//! 或启用内置广告/追踪器域名黑名单（约 3500 域）。

mod ad_domains;
mod blocker;

pub use ad_domains::AD_DOMAINS;
pub use blocker::DomainBlocker;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_exact_domain() {
        let mut blocker = DomainBlocker::new();
        blocker.block_domain("ads.example.com");

        assert!(blocker.should_block("https://ads.example.com/banner.js"));
        assert!(blocker.should_block("http://ads.example.com/"));
        assert!(!blocker.should_block("https://example.com/page"));
        assert!(!blocker.should_block("https://notads.example.com/"));
    }

    #[test]
    fn test_block_subdomain() {
        let mut blocker = DomainBlocker::new();
        blocker.block_domain("tracker.io");

        assert!(blocker.should_block("https://tracker.io/pixel"));
        assert!(blocker.should_block("https://sub.tracker.io/event"));
        assert!(blocker.should_block("https://a.b.tracker.io/x"));
        assert!(!blocker.should_block("https://nottracker.io/"));
    }

    #[test]
    fn test_block_domains_batch() {
        let mut blocker = DomainBlocker::new();
        blocker.block_domains(&["ads.com", "track.net", "spam.org"]);

        assert!(blocker.should_block("https://ads.com/ad.js"));
        assert!(blocker.should_block("https://track.net/pixel"));
        assert!(blocker.should_block("https://spam.org/popup"));
        assert!(!blocker.should_block("https://safe.com/"));
    }

    #[test]
    fn test_ad_blocking() {
        let mut blocker = DomainBlocker::new();
        assert!(!blocker.is_ad_blocking_enabled());

        blocker.enable_ad_blocking();
        assert!(blocker.is_ad_blocking_enabled());
        assert!(blocker.len() > 100, "应加载 100+ 广告域名");

        // 验证常见广告域被拦截
        assert!(blocker.should_block("https://doubleclick.net/ad"));
        assert!(blocker.should_block("https://google-analytics.com/collect"));
        assert!(blocker.should_block("https://cdn.criteo.com/js/ld/ld.js"));
        assert!(blocker.should_block("https://hm.baidu.com/hm.js"));

        // 正常网站不被拦截
        assert!(!blocker.should_block("https://github.com/"));
        assert!(!blocker.should_block("https://rust-lang.org/"));
    }

    #[test]
    fn test_should_block_host() {
        let mut blocker = DomainBlocker::new();
        blocker.block_domain("ads.example.com");

        assert!(blocker.should_block_host("ads.example.com"));
        assert!(blocker.should_block_host("sub.ads.example.com"));
        assert!(!blocker.should_block_host("example.com"));
    }

    #[test]
    fn test_cdp_patterns() {
        let mut blocker = DomainBlocker::new();
        blocker.block_domain("ads.example.com");
        blocker.block_domain("tracker.io");

        let patterns = blocker.to_cdp_patterns();
        assert_eq!(patterns.len(), 2);
        // 验证格式
        let p = &patterns[0];
        assert!(p["urlPattern"].as_str().unwrap().contains("*://*."));
        assert_eq!(p["requestStage"], "Request");
    }

    #[test]
    fn test_case_insensitive() {
        let mut blocker = DomainBlocker::new();
        blocker.block_domain("ADS.Example.COM");

        assert!(blocker.should_block("https://ads.example.com/ad.js"));
        assert!(blocker.should_block("https://ADS.EXAMPLE.COM/"));
    }

    #[test]
    fn test_invalid_url_not_blocked() {
        let mut blocker = DomainBlocker::new();
        blocker.block_domain("ads.com");

        assert!(!blocker.should_block("not-a-url"));
        assert!(!blocker.should_block(""));
    }

    #[test]
    fn blocked_domains_returns_wildcard_patterns() {
        let mut blocker = DomainBlocker::new();
        blocker.block_domains(&["ads.example.com", "tracker.io"]);
        let mut patterns = blocker.blocked_domains();
        patterns.sort();
        assert_eq!(patterns, vec!["*://ads.example.com/*", "*://tracker.io/*"]);
    }
}
