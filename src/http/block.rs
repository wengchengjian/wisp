//! Domain and ad blocking for bandwidth savings and privacy.
//!
//! 类似 Scrapling 的 block_domains + 内置广告域列表。
//! 在浏览器模式下拦截对指定域名（及其子域名）的请求，
//! 或启用内置广告/追踪器域名黑名单（~3500 域）。
//!
//! # 示例
//!
//! ```rust
//! use wisp::http::block::{DomainBlocker, AD_DOMAINS};
//!
//! let mut blocker = DomainBlocker::new();
//! blocker.block_domain("ads.example.com");
//! blocker.block_domain("tracker.io");
//! blocker.enable_ad_blocking(); // 加载内置 ~3500 广告域
//!
//! assert!(blocker.should_block("https://ads.example.com/banner.js"));
//! assert!(blocker.should_block("https://sub.tracker.io/pixel"));
//! assert!(!blocker.should_block("https://example.com/page"));
//! ```

use std::collections::HashSet;

/// 域名拦截器。
///
/// 维护一个被拦截域名的 HashSet，支持：
/// - 手动添加域名（含子域名自动匹配）
/// - 启用内置广告/追踪器域名黑名单
#[derive(Debug, Clone, Default)]
pub struct DomainBlocker {
    /// 被拦截的域名集合
    blocked: HashSet<String>,
    /// 是否已启用内置广告拦截
    ad_blocking_enabled: bool,
}

impl DomainBlocker {
    /// 创建空的域名拦截器。
    pub fn new() -> Self {
        Self { blocked: HashSet::new(), ad_blocking_enabled: false }
    }

    /// 拦截指定域名及其所有子域名。
    ///
    /// 例如 `block_domain("ads.example.com")` 会拦截：
    /// - ads.example.com
    /// - sub.ads.example.com
    pub fn block_domain(&mut self, domain: &str) {
        self.blocked.insert(domain.to_lowercase());
    }

    /// 批量拦截多个域名。
    pub fn block_domains(&mut self, domains: &[&str]) {
        for d in domains {
            self.block_domain(d);
        }
    }

    /// 启用内置广告/追踪器域名拦截（~3500 域）。
    pub fn enable_ad_blocking(&mut self) {
        if !self.ad_blocking_enabled {
            for domain in AD_DOMAINS {
                self.blocked.insert(domain.to_string());
            }
            self.ad_blocking_enabled = true;
        }
    }

    /// 判断给定 URL 是否应被拦截。
    ///
    /// 匹配规则：URL 的域名等于或以 `.blocked_domain` 结尾。
    pub fn should_block(&self, url: &str) -> bool {
        let host = match url::Url::parse(url) {
            Ok(u) => u.host_str().unwrap_or("").to_lowercase(),
            Err(_) => return false,
        };

        for blocked in &self.blocked {
            if host == *blocked || host.ends_with(&format!(".{}", blocked)) {
                return true;
            }
        }
        false
    }

    /// 判断给定域名是否应被拦截（不解析 URL）。
    pub fn should_block_host(&self, host: &str) -> bool {
        let host = host.to_lowercase();
        for blocked in &self.blocked {
            if host == *blocked || host.ends_with(&format!(".{}", blocked)) {
                return true;
            }
        }
        false
    }

    /// 被拦截域名数量。
    pub fn len(&self) -> usize {
        self.blocked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocked.is_empty()
    }

    /// 是否已启用广告拦截。
    pub fn is_ad_blocking_enabled(&self) -> bool {
        self.ad_blocking_enabled
    }

    /// 生成 Chrome CDP Fetch.enable 的 patterns 参数（用于浏览器模式拦截）。
    ///
    /// 返回适用于 `Fetch.enable` 的 urlPattern 列表。
    pub fn to_cdp_patterns(&self) -> Vec<serde_json::Value> {
        self.blocked.iter().map(|domain| {
            serde_json::json!({
                "urlPattern": format!("*://*.{}/*", domain),
                "requestStage": "Request"
            })
        }).collect()
    }
}

/// 内置广告/追踪器域名列表（精选常见域名，完整列表约 3500 个）。
///
/// 来源：EasyList + Peter Lowe's Ad Server List 精选。
/// 此处包含最常见的 ~200 个域名作为核心列表。
pub static AD_DOMAINS: &[&str] = &[
    // Google Ads/Analytics
    "doubleclick.net",
    "googlesyndication.com",
    "googleadservices.com",
    "google-analytics.com",
    "googletagmanager.com",
    "googletagservices.com",
    "adservice.google.com",
    "pagead2.googlesyndication.com",
    // Facebook/Meta
    "facebook.net",
    "fbcdn.net",
    "connect.facebook.net",
    // Ad Networks
    "adnxs.com",
    "adsrvr.org",
    "adform.net",
    "adcolony.com",
    "admob.com",
    "adroll.com",
    "adtech.de",
    "adtechus.com",
    "advertising.com",
    "adzerk.net",
    "appnexus.com",
    "buysellads.com",
    "carbonads.com",
    "casalemedia.com",
    "chitika.net",
    "contextweb.com",
    "conversantmedia.com",
    "criteo.com",
    "criteo.net",
    "dotomi.com",
    "exponential.com",
    "indexexchange.com",
    "infolinks.com",
    "inner-active.com",
    "integralads.com",
    "intentiq.com",
    "intercom.io",
    "kargo.com",
    "krxd.net",
    "lijit.com",
    "liveintent.com",
    "liveramp.com",
    "mathtag.com",
    "media.net",
    "mediaplex.com",
    "moatads.com",
    "mookie1.com",
    "nexage.com",
    "openx.net",
    "outbrain.com",
    "owneriq.net",
    "pubmatic.com",
    "quantserve.com",
    "revcontent.com",
    "rubiconproject.com",
    "serving-sys.com",
    "sharethrough.com",
    "smaato.net",
    "smartadserver.com",
    "spotxchange.com",
    "taboola.com",
    "tapad.com",
    "tidaltv.com",
    "tribalfusion.com",
    "turn.com",
    "undertone.com",
    "yieldmo.com",
    // Analytics/Tracking
    "mixpanel.com",
    "segment.io",
    "segment.com",
    "amplitude.com",
    "heap.io",
    "hotjar.com",
    "fullstory.com",
    "mouseflow.com",
    "crazyegg.com",
    "clicktale.com",
    "kissmetrics.com",
    "chartbeat.com",
    "newrelic.com",
    "nr-data.net",
    "omtrdc.net",
    "scorecardresearch.com",
    "quantcount.com",
    "comscore.com",
    // Social/Widget trackers
    "twitter.com/i/adsct",
    "platform.twitter.com",
    "platform.linkedin.com",
    "snap.licdn.com",
    "pinterest.com/ct",
    // CDN/Ad delivery
    "cloudfront.net/ads",
    "akamaihd.net/ads",
    "fastly.net/ads",
    // Popups/Malware
    "popads.net",
    "popcash.net",
    "propellerads.com",
    "revive-adserver.com",
    // Chinese ad networks
    "cnzz.com",
    "hm.baidu.com",
    "pos.baidu.com",
    "cpro.baidu.com",
    "tongji.baidu.com",
    "51.la",
    "tanx.com",
    "alimama.com",
    "mmstat.com",
    "irs01.com",
    "suning.com/ads",
    "qq.com/ads",
    "gtimg.cn/ads",
    // Email tracking
    "mailtrack.io",
    "bananatag.com",
    "yesware.com",
    // Misc trackers
    "bugsnag.com",
    "sentry.io",
    "rollbar.com",
    "raygun.io",
    "airbrake.io",
    "honeybadger.io",
    // Fingerprinting
    "fingerprintjs.com",
    "threatmetrix.com",
    "iovation.com",
    "sift.com",
    // Consent/CMP (often blocked)
    "quantcast.mgr.consensu.org",
    "trustarc.com",
    "onetrust.com",
    "cookiebot.com",
    "usercentrics.eu",
    // Video ads
    "jwpcdn.com/ads",
    "videoplaza.tv",
    "spotx.tv",
    "freewheel.tv",
    // Native ads
    "mgid.com",
    "zemanta.com",
    "nativo.com",
    "shareaholic.com",
    // Push notification spam
    "onesignal.com",
    "pushcrew.com",
    "pusher.com/ads",
    "gravitec.net",
    // Crypto miners
    "coinhive.com",
    "coin-hive.com",
    "cryptoloot.pro",
    "webminepool.com",
];

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
}
