//! Fetcher — 根据 FetchMode 委托给 FetchClient 的一次性请求入口。

use crate::FetcherBuilder;
use crate::client::{FetchClient, FetchClientConfig};
use std::sync::Arc;

use wisp_core::error::{Result, WispError};
use wisp_core::{FetchMode, Request, Response};

/// Fetcher — FetchClient 的薄包装，用于一次性请求场景。
///
/// 持有 `Arc<FetchClient>`，所有请求委托给 FetchClient。
/// HTTP 请求共享连接池，浏览器请求通过 BrowserPool 复用实例。
pub struct Fetcher {
    client: Arc<FetchClient>,
    mode: FetchMode,
}

impl Fetcher {
    /// 快速 HTTP 模式（TLS 指纹，毫秒级）。
    #[must_use]
    pub fn http() -> FetcherBuilder {
        FetcherBuilder::new(FetchMode::Http)
    }

    /// 浏览器渲染模式（JS 执行，秒级）。
    #[must_use]
    pub fn dynamic() -> FetcherBuilder {
        FetcherBuilder::new(FetchMode::Dynamic)
    }

    /// 隐身模式（CF bypass，秒级）。
    #[must_use]
    pub fn stealth() -> FetcherBuilder {
        FetcherBuilder::new(FetchMode::Stealth)
    }

    /// 从已有 FetchClient 创建 Fetcher。
    pub fn from_client(client: Arc<FetchClient>, mode: FetchMode) -> Result<Self> {
        if mode == FetchMode::Auto {
            return Err(WispError::Config(
                "Auto mode is owned by wisp_crawl Engine; use Http/Dynamic/Stealth explicitly"
                    .into(),
            ));
        }
        Ok(Self { client, mode })
    }

    /// 从配置创建 Fetcher。
    pub fn new(mode: FetchMode, config: FetchClientConfig) -> Result<Self> {
        // Auto 是 crawl 层的升级策略，不属于一次性 Fetcher 的执行语义。
        if mode == FetchMode::Auto {
            return Err(WispError::Config(
                "Auto mode is owned by wisp_crawl Engine; use Http/Dynamic/Stealth explicitly"
                    .into(),
            ));
        }
        let client = Arc::new(FetchClient::new(config)?);
        Ok(Self { client, mode })
    }

    /// 获取当前模式。
    #[must_use]
    pub fn mode(&self) -> FetchMode {
        self.mode
    }

    /// 获取底层 FetchClient 引用。
    #[must_use]
    pub fn client(&self) -> &FetchClient {
        &self.client
    }

    /// 获取配置引用。
    #[must_use]
    pub fn config(&self) -> &FetchClientConfig {
        self.client.config()
    }

    /// GET 请求。
    ///
    /// # Errors
    ///
    /// 网络请求失败时返回 `WispError::Network`（DNS/TLS/超时/代理等）。
    pub async fn get(&self, url: &str) -> Result<Response> {
        self.fetch(Request::get(url)).await
    }

    /// POST 请求。
    pub async fn post(&self, url: &str, body: Option<&str>) -> Result<Response> {
        let mut req = Request::post(url, body.map(std::string::ToString::to_string));
        req.headers = self.config().headers.clone();
        self.fetch(req).await
    }

    /// 发送请求（根据模式委托给 FetchClient）。
    pub async fn fetch(&self, req: Request) -> Result<Response> {
        self.client.fetch(&req, self.mode).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{FetchClient, FetchClientConfig};
    use std::sync::Arc;

    #[test]
    fn from_client_http_works() {
        let client = Arc::new(
            FetchClient::new(FetchClientConfig {
                max_concurrent_pages: 0,
                ..Default::default()
            })
            .unwrap(),
        );
        let fetcher = Fetcher::from_client(client, FetchMode::Http).unwrap();
        assert_eq!(fetcher.mode(), FetchMode::Http);
    }

    #[cfg(feature = "browser")]
    #[test]
    fn from_client_dynamic_builds_strategy() {
        let client = Arc::new(FetchClient::new(FetchClientConfig::default()).unwrap());
        let fetcher = Fetcher::from_client(client, FetchMode::Dynamic).unwrap();
        assert_eq!(fetcher.mode(), FetchMode::Dynamic);
    }

    #[cfg(feature = "stealth")]
    #[test]
    fn stealth_builder_forces_headed_offscreen_even_when_user_requests_headless() {
        let fetcher = Fetcher::stealth().headless(true).build().unwrap();
        assert!(fetcher.config().force_headed_offscreen);
        assert!(
            fetcher.config().headless,
            "用户配置保留，运行层才临时覆盖为 headed"
        );
    }

    #[cfg(feature = "browser")]
    #[test]
    fn dynamic_builder_keeps_user_headless_choice() {
        let headless = Fetcher::dynamic().headless(true).build().unwrap();
        assert!(!headless.config().force_headed_offscreen);
        assert!(headless.config().headless);

        let headed = Fetcher::dynamic().headless(false).build().unwrap();
        assert!(!headed.config().force_headed_offscreen);
        assert!(!headed.config().headless);
    }
}
