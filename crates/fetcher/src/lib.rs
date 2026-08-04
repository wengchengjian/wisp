//! 统一 Fetcher 入口 - 根据 FetchMode 委托给 FetchClient。
//!
//! Fetcher 是 FetchClient 的薄包装，用于一次性请求场景。
//! 持续爬取场景应直接使用 FetchClient。
//!
//! # 三模式
//!
//! - `FetchMode::Http` - 快速 HTTP（TLS 指纹模拟，毫秒级，无浏览器）
//! - `FetchMode::Dynamic` - 浏览器渲染（JS 执行，秒级）
//! - `FetchMode::Stealth` - 隐身浏览器（CF bypass + 人类行为模拟，秒级）
//!
//! # 示例
//!
//! ```rust,no_run
//! use wisp_fetcher::Fetcher;
//! use wisp_parser::ResponseExt;
//!
//! # async fn example() -> wisp_core::error::Result<()> {
//! // 三模式，统一 API
//! let page = Fetcher::http().get("https://quotes.toscrape.com/").await?;
//! let quotes = page.css(".quote");
//!
//! let page = Fetcher::stealth()
//!     .proxy("http://127.0.0.1:7897")
//!     .get("https://cf-protected.com/")
//!     .await?;
//! let data = page.css(".content");
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod cookie;
#[cfg(feature = "browser")]
pub mod strategy;

mod builder;
mod fetcher;

#[cfg(feature = "browser")]
pub use strategy::DynamicStrategy;
#[cfg(feature = "stealth")]
pub use strategy::StealthStrategy;

pub use builder::FetcherBuilder;
pub use client::{FetchClient, FetchClientConfig, FetchOptions};
pub use fetcher::Fetcher;
pub use wisp_core::{CrawlRequest, FetchMode, Method, Request, Response};

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wreq_util::Profile;

    #[test]
    fn test_fetch_mode_enum() {
        assert_ne!(FetchMode::Http, FetchMode::Dynamic);
        assert_ne!(FetchMode::Dynamic, FetchMode::Stealth);
        assert_eq!(FetchMode::Http, FetchMode::Http);
    }

    #[test]
    fn test_fetcher_builder_http() {
        let fetcher = Fetcher::http()
            .proxy("http://127.0.0.1:7897")
            .timeout(Duration::from_mins(1))
            .emulation(Profile::Firefox128)
            .build()
            .expect("build fetcher");

        assert_eq!(fetcher.mode(), FetchMode::Http);
        assert_eq!(
            fetcher.config().proxy.as_deref(),
            Some("http://127.0.0.1:7897")
        );
        assert_eq!(fetcher.config().timeout, Duration::from_mins(1));
        assert_eq!(fetcher.config().emulation, Some(Profile::Firefox128));
    }

    #[cfg(feature = "stealth")]
    #[test]
    fn test_fetcher_builder_stealth() {
        let fetcher = Fetcher::stealth()
            .headless(true)
            .human_mode(true)
            .challenge_timeout(Duration::from_mins(1))
            .proxy("http://127.0.0.1:7897")
            .build()
            .expect("build fetcher");

        assert_eq!(fetcher.mode(), FetchMode::Stealth);
        assert!(fetcher.config().headless);
        assert!(fetcher.config().human_mode);
        assert_eq!(fetcher.config().challenge_timeout, Duration::from_mins(1));
    }

    #[cfg(feature = "browser")]
    #[test]
    fn test_fetcher_builder_dynamic() {
        let fetcher = Fetcher::dynamic()
            .headless(false)
            .wait_for(".content")
            .extra_wait_ms(2000)
            .build()
            .expect("build fetcher");

        assert_eq!(fetcher.mode(), FetchMode::Dynamic);
        assert!(!fetcher.config().headless);
        assert_eq!(fetcher.config().wait_for.as_deref(), Some(".content"));
        assert_eq!(fetcher.config().extra_wait_ms, 2000);
    }

    #[cfg(feature = "browser")]
    #[test]
    fn test_fetcher_builder_block_ads() {
        let fetcher = Fetcher::dynamic()
            .block_ads()
            .block_domains(&["analytics.example.com"])
            .build()
            .expect("build fetcher");

        let blocker = fetcher.config().domain_blocker.as_ref().unwrap();
        assert!(blocker.is_ad_blocking_enabled());
        assert!(blocker.should_block("https://analytics.example.com/track"));
    }

    #[test]
    fn test_fetcher_config_default() {
        let config = FetchClientConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert!(config.headless);
        assert!(config.human_mode);
        assert_eq!(config.emulation, Some(Profile::Chrome136));
        assert!(config.proxy.is_none());
        assert!(config.domain_blocker.is_none());
    }

    #[test]
    fn test_fetcher_http_mode_has_no_strategy() {
        let fetcher =
            Fetcher::new(FetchMode::Http, FetchClientConfig::default()).expect("build fetcher");
        assert_eq!(fetcher.mode(), FetchMode::Http);
    }

    #[test]
    fn test_fetcher_auto_mode_has_no_strategy() {
        let err = match Fetcher::new(FetchMode::Auto, FetchClientConfig::default()) {
            Ok(_) => panic!("Auto 应由 crawl Engine 处理"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("Auto"));
    }

    #[cfg(feature = "browser")]
    #[test]
    fn test_fetcher_dynamic_mode_has_strategy() {
        let fetcher =
            Fetcher::new(FetchMode::Dynamic, FetchClientConfig::default()).expect("build fetcher");
        assert_eq!(fetcher.mode(), FetchMode::Dynamic);
    }

    #[cfg(feature = "stealth")]
    #[test]
    fn test_fetcher_stealth_mode_has_strategy() {
        let fetcher =
            Fetcher::new(FetchMode::Stealth, FetchClientConfig::default()).expect("build fetcher");
        assert_eq!(fetcher.mode(), FetchMode::Stealth);
    }
}
