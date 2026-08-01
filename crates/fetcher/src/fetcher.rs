//! Fetcher — 根据 FetchMode 委托给 FetchClient 的一次性请求入口。

use crate::client::{FetchClient, FetchClientConfig};
use crate::FetcherBuilder;
use std::sync::Arc;

use wisp_core::error::{Result, WispError};
use wisp_core::{FetchMode, Request, Response};

#[cfg(feature = "browser")]
use crate::cookie::CfCookieJar;
#[cfg(feature = "browser")]
use crate::strategies::DynamicStrategy;
#[cfg(feature = "stealth")]
use crate::strategies::StealthStrategy;
#[cfg(feature = "browser")]
use crate::strategy::BrowserFetchStrategy;

/// Fetcher — FetchClient 的薄包装，用于一次性请求场景。
///
/// 持有 `Arc<FetchClient>`，所有请求委托给 FetchClient。
/// HTTP 请求共享连接池，浏览器请求通过 BrowserPool 复用实例。
pub struct Fetcher {
    client: Arc<FetchClient>,
    mode: FetchMode,
    /// 浏览器模式下的 strategy（Http/Auto 为 None）。
    /// ARCH: 由 Fetcher::new 根据 mode 自动构造。
    #[cfg(feature = "browser")]
    browser_strategy: Option<Arc<dyn BrowserFetchStrategy>>,
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
    /// 注意：此构造方式不创建 browser_strategy，Dynamic/Stealth 模式下需调用方自行注入。
    #[must_use]
    pub fn from_client(client: Arc<FetchClient>, mode: FetchMode) -> Self {
        Self {
            client,
            mode,
            #[cfg(feature = "browser")]
            browser_strategy: None,
        }
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
        let client = Arc::new(FetchClient::new(config.clone())?);
        #[cfg(feature = "browser")]
        {
            let browser_strategy = Self::build_strategy(mode, &config)?;
            Ok(Self {
                client,
                mode,
                browser_strategy,
            })
        }
        #[cfg(not(feature = "browser"))]
        {
            // 非 browser feature 下，Dynamic/Stealth 模式不可用
            match mode {
                FetchMode::Http => Ok(Self { client, mode }),
                FetchMode::Auto => Err(WispError::Config(
                    "Auto mode is owned by wisp_crawl Engine".into(),
                )),
                FetchMode::Dynamic | FetchMode::Stealth => Err(WispError::Config(format!(
                    "{mode:?} mode requires 'browser' feature"
                ))),
            }
        }
    }

    /// 根据 mode 构造 browser_strategy。
    #[cfg(feature = "browser")]
    fn build_strategy(
        mode: FetchMode,
        config: &FetchClientConfig,
    ) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        match mode {
            FetchMode::Http => Ok(None),
            FetchMode::Auto => Err(WispError::Config(
                "Auto mode is owned by wisp_crawl Engine".into(),
            )),
            FetchMode::Dynamic => Self::build_dynamic_strategy(config),
            FetchMode::Stealth => Self::build_stealth_strategy(config),
        }
    }

    #[cfg(feature = "browser")]
    fn build_dynamic_strategy(
        config: &FetchClientConfig,
    ) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        Ok(Some(Arc::new(DynamicStrategy::from_config(config))))
    }

    #[cfg(feature = "stealth")]
    fn build_stealth_strategy(
        config: &FetchClientConfig,
    ) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
        Ok(Some(Arc::new(StealthStrategy::from_config(config, cf_jar))))
    }

    #[cfg(all(feature = "browser", not(feature = "stealth")))]
    fn build_stealth_strategy(
        _: &FetchClientConfig,
    ) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        Err(WispError::Config(
            "Stealth mode requires 'stealth' feature".into(),
        ))
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

    /// 获取浏览器策略引用（如有）。
    #[must_use]
    #[cfg(feature = "browser")]
    pub fn browser_strategy(&self) -> Option<&Arc<dyn BrowserFetchStrategy>> {
        self.browser_strategy.as_ref()
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
        match self.mode {
            FetchMode::Http => self.client.fetch_http(&req).await,
            FetchMode::Auto => Err(WispError::Config(
                "Auto mode is owned by wisp_crawl Engine; use Http/Dynamic/Stealth explicitly"
                    .into(),
            )),
            FetchMode::Dynamic | FetchMode::Stealth => {
                #[cfg(feature = "browser")]
                {
                    let strategy = self.browser_strategy.as_ref().ok_or_else(|| {
                        WispError::Config(format!(
                            "{:?} mode requires browser_strategy, use Fetcher::new() instead of from_client()",
                            self.mode
                        ))
                    })?;
                    self.client.fetch_browser(&req, strategy.as_ref()).await
                }
                #[cfg(not(feature = "browser"))]
                {
                    Err(WispError::Config(format!(
                        "{:?} mode requires 'browser' feature",
                        self.mode
                    )))
                }
            }
        }
    }
}
