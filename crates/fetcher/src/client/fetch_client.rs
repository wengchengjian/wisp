//! FetchClient 实现：封装 HTTP Client 与 BrowserPool。

use super::config::FetchClientConfig;
use std::sync::Arc;
#[cfg(feature = "browser")]
use std::time::Duration;

use crate::cookie::{CookieJar, HttpCookieJar};
#[cfg(feature = "browser")]
use crate::strategy::BrowserFetchStrategy;
#[cfg(feature = "browser")]
use wisp_browser::BrowserPool;
#[cfg(feature = "browser")]
use wisp_core::config::LaunchOptions;
use wisp_core::error::{Result, WispError};
use wisp_core::{Request, Response};
use wisp_http::Client;

/// 统一请求客户端：封装 HTTP Client 和 BrowserPool。
///
/// - HTTP 请求：共享 `http::Client`（连接池复用）
/// - 浏览器请求：通过 `BrowserPool`（实例复用，RAII 自动归还）
/// - Cookie 管理：通过 `cookie_jar` 统一 HTTP/浏览器/CF cookie 状态
pub struct FetchClient {
    http: Arc<Client>,
    #[cfg(feature = "browser")]
    browser_pool: Option<Arc<BrowserPool>>,
    config: FetchClientConfig,
    /// 共享 cookie jar（默认 HttpCookieJar，StealthStrategy 可注入 CfCookieJar）
    cookie_jar: Arc<dyn CookieJar>,
}

impl FetchClient {
    /// 创建 FetchClient。
    pub fn new(config: FetchClientConfig) -> Result<Self> {
        let http_jar = Arc::new(HttpCookieJar::new());
        let http = Arc::new(Self::build_http_client(&config, http_jar.jar())?);
        #[cfg(feature = "browser")]
        let browser_pool = Self::build_browser_pool(&config)?;
        let cookie_jar: Arc<dyn CookieJar> = http_jar;
        Ok(Self {
            http,
            #[cfg(feature = "browser")]
            browser_pool,
            config,
            cookie_jar,
        })
    }

    /// 获取共享 CookieJar。
    #[must_use]
    pub fn cookie_jar(&self) -> &Arc<dyn CookieJar> {
        &self.cookie_jar
    }

    /// 获取 HTTP 客户端引用。
    #[must_use]
    pub fn http(&self) -> &Client {
        &self.http
    }

    /// 获取 HTTP 客户端的 Arc 克隆（用于需要独立持有 Client 的中间件，如 RobotsMiddleware）。
    #[must_use]
    pub fn http_arc(&self) -> Arc<Client> {
        Arc::clone(&self.http)
    }

    /// 获取浏览器池引用（若有）。
    #[must_use]
    #[cfg(feature = "browser")]
    pub fn browser_pool(&self) -> Option<&Arc<BrowserPool>> {
        self.browser_pool.as_ref()
    }

    /// 获取配置引用。
    #[must_use]
    pub fn config(&self) -> &FetchClientConfig {
        &self.config
    }

    /// HTTP 请求（共享 Client，连接复用）。直接返回统一 Response，无中间类型转换。
    pub async fn fetch_http(&self, req: &Request) -> Result<Response> {
        if let Some(ref blocker) = self.config.domain_blocker {
            if blocker.should_block(&req.url) {
                return Err(WispError::Config(format!(
                    "domain blocked by DomainBlocker: {}",
                    wisp_core::utils::sanitize_url(&req.url)
                )));
            }
        }
        self.http.fetch(req).await
    }

    /// 浏览器请求（通过 BrowserPool + 注入 strategy）。
    ///
    /// ARCH: 替代 `fetch_browser(&req, solve_cf: bool)`。
    /// strategy 由调用方传入，FetchClient 不再关心 CF/Dynamic 差异。
    /// 120s 总超时由本方法包装。
    #[cfg(feature = "browser")]
    pub async fn fetch_browser(
        &self,
        req: &Request,
        strategy: &dyn BrowserFetchStrategy,
    ) -> Result<Response> {
        let pool = self.browser_pool.as_ref().ok_or_else(|| {
            WispError::Browser(wisp_core::error::BrowserError::Other(
                "browser pool not configured (max_concurrent_pages=0)".into(),
            ))
        })?;
        // acquire 返回带 page 的 handle（permit 限制并发数）
        let mut handle = pool.acquire().await?;

        if let Some(ref blocker) = self.config.domain_blocker {
            if blocker.should_block(&req.url) {
                return Err(WispError::Config(format!(
                    "domain blocked by DomainBlocker: {}",
                    wisp_core::utils::sanitize_url(&req.url)
                )));
            }
            let urls = blocker.blocked_domains();
            if !urls.is_empty() {
                handle
                    .page_mut()
                    .cmd(
                        "Network.setBlockedURLs",
                        serde_json::json!({ "urls": urls }),
                    )
                    .await?;
            }
        }

        // 总超时：防止 CF 挑战页面卡住整个流程（导航+挑战+提取各阶段都有单独超时，
        // 但极端情况下可能累加超过预期，这里加一个 120s 硬上限）
        let work = strategy.fetch(handle.page_mut(), req);
        let result = tokio::time::timeout(Duration::from_secs(120), work)
            .await
            .map_err(|_| {
                WispError::Timeout(format!(
                    "fetch_browser 总超时（120s）: {}",
                    wisp_core::utils::sanitize_url(&req.url)
                ))
            })?;

        // 实际工作；无论成功/失败都显式关闭 tab
        let _ = handle.page_mut().close().await;
        // handle Drop：page.target_id 已 None（Page::Drop no-op）+ permit 自动 release
        result
    }

    fn build_http_client(
        config: &FetchClientConfig,
        cookie_jar: Arc<wreq::cookie::Jar>,
    ) -> Result<Client> {
        // ND-008-SEC/ND-011-SEC：将 max_response_size 和 danger_accept_invalid_certs
        // 传递给底层 http::Client，使配置实际生效。
        let mut builder = Client::builder()
            .timeout(config.timeout)
            .max_redirects(config.max_redirects)
            .max_body_size(config.max_body_size)
            .danger_accept_invalid_certs(config.danger_accept_invalid_certs)
            .cookie_provider(cookie_jar);

        if let Some(ref proxy) = config.proxy {
            builder = builder.proxy(proxy);
        }
        if let Some(ref ua) = config.user_agent {
            builder = builder.user_agent(ua);
        }
        if let Some(emu) = config.emulation {
            builder = builder.emulation(emu);
        } else {
            builder = builder.no_emulation();
        }
        for (k, v) in &config.headers {
            builder = builder.header(k, v);
        }
        builder.build()
    }

    #[cfg(feature = "browser")]
    fn build_browser_pool(config: &FetchClientConfig) -> Result<Option<Arc<BrowserPool>>> {
        if config.max_concurrent_pages == 0 {
            return Ok(None);
        }
        let proxy_config = match &config.proxy {
            Some(proxy) => {
                let parsed = url::Url::parse(proxy)
                    .map_err(|e| WispError::Config(format!("invalid proxy URL: {e}")))?;
                if !parsed.username().is_empty() || parsed.password().is_some() {
                    return Err(WispError::Config("浏览器模式暂不支持代理认证".into()));
                }
                Some(wisp_core::config::ProxyConfig {
                    server: proxy.clone(),
                    username: None,
                    password: None,
                })
            }
            None => None,
        };
        let launch_options = LaunchOptions {
            headless: config.headless,
            executable_path: config.executable_path.clone(),
            proxy: proxy_config,
            ..Default::default()
        };
        Ok(Some(BrowserPool::new(
            config.max_concurrent_pages,
            launch_options,
        )))
    }
}

#[cfg(feature = "browser")]
impl Drop for FetchClient {
    fn drop(&mut self) {
        let Some(pool) = self.browser_pool.take() else {
            return;
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let _ = handle.spawn(async move {
                pool.shutdown().await;
            });
        }
    }
}
