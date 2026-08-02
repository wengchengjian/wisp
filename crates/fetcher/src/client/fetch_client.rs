//! FetchClient 实现：封装 HTTP Client 与 BrowserPool。

use super::config::FetchClientConfig;
use moka::sync::Cache;
use std::sync::Arc;
#[cfg(feature = "browser")]
use std::time::Duration;

#[cfg(feature = "stealth")]
use crate::cookie::CfCookieJar;
use crate::cookie::CompositeCookieJar;
use crate::cookie::{CookieJar, HttpCookieJar};
#[cfg(feature = "browser")]
use crate::strategies::DynamicStrategy;
#[cfg(feature = "stealth")]
use crate::strategies::StealthStrategy;
#[cfg(feature = "browser")]
use crate::strategy::BrowserFetchStrategy;
#[cfg(feature = "browser")]
use wisp_browser::BrowserPool;
#[cfg(feature = "browser")]
use wisp_core::config::LaunchOptions;
use wisp_core::error::{Result, WispError};
use wisp_core::{FetchMode, Request, Response};
use wisp_http::Client;
use wreq_util::Profile;

/// 统一请求客户端：封装 HTTP Client 和 BrowserPool。
///
/// 对外深 seam 是 [`FetchClient::fetch`]；per-request proxy 与 cookie 状态都在内部管理。
pub struct FetchClient {
    http: Arc<Client>,
    http_jar: Arc<HttpCookieJar>,
    proxy_clients: Cache<String, Arc<Client>>,
    #[cfg(feature = "browser")]
    browser_pool: Option<Arc<BrowserPool>>,
    #[cfg(feature = "browser")]
    dynamic_strategy: Option<Arc<dyn BrowserFetchStrategy>>,
    #[cfg(feature = "stealth")]
    stealth_strategy: Option<Arc<dyn BrowserFetchStrategy>>,
    config: FetchClientConfig,
    cookie_jar: Arc<dyn CookieJar>,
}

impl FetchClient {
    /// 创建 FetchClient。
    pub fn new(config: FetchClientConfig) -> Result<Self> {
        let http_jar = Arc::new(HttpCookieJar::new());
        let http = Arc::new(Self::build_http_client(&config, http_jar.jar())?);
        let proxy_clients = Cache::builder().max_capacity(1024).build();
        #[cfg(feature = "browser")]
        let browser_pool = Self::build_browser_pool(&config)?;
        #[cfg(feature = "stealth")]
        let cookie_jar: Arc<dyn CookieJar> = {
            let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
            Arc::new(CompositeCookieJar::new(http_jar.clone(), cf_jar))
        };
        #[cfg(not(feature = "stealth"))]
        let cookie_jar: Arc<dyn CookieJar> = Arc::new(CompositeCookieJar::new(http_jar.clone()));
        #[cfg(feature = "browser")]
        let dynamic_strategy =
            Some(Arc::new(DynamicStrategy::from_config(&config)) as Arc<dyn BrowserFetchStrategy>);
        #[cfg(feature = "stealth")]
        let stealth_strategy = Some(Arc::new(StealthStrategy::from_config(
            &config,
            cookie_jar.clone(),
        )) as Arc<dyn BrowserFetchStrategy>);
        Ok(Self {
            http,
            http_jar,
            proxy_clients,
            #[cfg(feature = "browser")]
            browser_pool,
            #[cfg(feature = "browser")]
            dynamic_strategy,
            #[cfg(feature = "stealth")]
            stealth_strategy,
            config,
            cookie_jar,
        })
    }

    /// 获取配置引用。
    #[must_use]
    pub fn config(&self) -> &FetchClientConfig {
        &self.config
    }

    /// 按模式分发请求；Auto 属于 crawl Engine，不由 FetchClient 处理。
    pub async fn fetch(&self, req: &Request, mode: FetchMode) -> Result<Response> {
        match mode {
            FetchMode::Http => self.fetch_http(req).await,
            FetchMode::Auto => Err(WispError::Config(
                "Auto mode is owned by wisp_crawl Engine; use Http/Dynamic/Stealth explicitly"
                    .into(),
            )),
            FetchMode::Dynamic | FetchMode::Stealth => self.fetch_browser_mode(req, mode).await,
        }
    }

    /// Auto 快速路径：若共享 cookie seam 已有会话 cookie，则走 HTTP。
    #[doc(hidden)]
    pub async fn fetch_http_with_cf_cookie(&self, req: &Request) -> Result<Option<Response>> {
        let Some(url_parsed) = url::Url::parse(&req.url).ok() else {
            return Ok(None);
        };
        let Some(cookie_header) = self.cookie_jar.header(&url_parsed).await else {
            return Ok(None);
        };
        let mut http_req = req.clone();
        http_req.headers.insert("Cookie".to_string(), cookie_header);
        if let Some(ua) = self.cookie_jar.ua(&url_parsed).await {
            http_req.headers.insert("User-Agent".to_string(), ua);
        }
        http_req.fetch_mode_override = Some(FetchMode::Http);
        let resp = self.fetch_http(&http_req).await?;
        if resp.status == 200 {
            let mut final_resp = resp;
            final_resp.request.fetch_mode_override = Some(FetchMode::Http);
            Ok(Some(final_resp))
        } else {
            Ok(None)
        }
    }

    pub(crate) async fn fetch_http(&self, req: &Request) -> Result<Response> {
        if let Some(ref blocker) = self.config.domain_blocker
            && blocker.should_block(&req.url)
        {
            return Err(WispError::Config(format!(
                "domain blocked by DomainBlocker: {}",
                wisp_core::utils::sanitize_url(&req.url)
            )));
        }
        if let Some(proxy) = req.proxy.as_deref() {
            let client = self.proxy_client(proxy)?;
            return client.fetch(req).await;
        }
        self.http.fetch(req).await
    }

    /// 使用共享 cookie/代理配置和指定 TLS 指纹执行 HTTP 抓取。
    ///
    /// MCP `fetch_page` 的 per-call emulation 需要独立 Client，因为 wreq 的
    /// TLS 指纹属于客户端级配置；共享 cookie jar 和 HTTP 配置仍从本客户端读取。
    pub async fn fetch_http_with_emulation(
        &self,
        req: &Request,
        emulation: Profile,
    ) -> Result<Response> {
        if let Some(ref blocker) = self.config.domain_blocker
            && blocker.should_block(&req.url)
        {
            return Err(WispError::Config(format!(
                "domain blocked by DomainBlocker: {}",
                wisp_core::utils::sanitize_url(&req.url)
            )));
        }
        let mut http = self.config.http.clone();
        http.emulation = Some(emulation);
        http.cookie_jar = Some(self.http_jar.jar());
        if let Some(proxy) = req.proxy.as_deref() {
            http.proxy = Some(proxy.to_string());
        }
        Client::from_config(http)?.fetch(req).await
    }

    fn proxy_client(&self, proxy: &str) -> Result<Arc<Client>> {
        if let Some(client) = self.proxy_clients.get(proxy) {
            return Ok(client);
        }
        let mut http = self.config.http.clone();
        http.proxy = Some(proxy.to_string());
        http.cookie_jar = Some(self.http_jar.jar());
        let client = Arc::new(Client::from_config(http)?);
        self.proxy_clients
            .insert(proxy.to_string(), Arc::clone(&client));
        Ok(client)
    }

    #[cfg(test)]
    pub(crate) fn cookie_jar(&self) -> &Arc<dyn CookieJar> {
        &self.cookie_jar
    }

    #[cfg(test)]
    pub(crate) fn http(&self) -> &Client {
        &self.http
    }

    #[cfg(feature = "browser")]
    #[cfg(all(test, feature = "browser"))]
    pub(crate) fn browser_pool(&self) -> Option<&Arc<BrowserPool>> {
        self.browser_pool.as_ref()
    }

    #[cfg(feature = "browser")]
    fn ensure_browser_proxy_allowed(&self, req: &Request) -> Result<()> {
        if let Some(proxy) = req.proxy.as_deref()
            && self.config.proxy.as_deref() != Some(proxy)
        {
            return Err(WispError::Config(format!(
                "browser mode does not support per-request proxy that differs from configured proxy: {proxy}"
            )));
        }
        Ok(())
    }

    #[cfg(feature = "browser")]
    async fn fetch_browser_mode(&self, req: &Request, mode: FetchMode) -> Result<Response> {
        self.ensure_browser_proxy_allowed(req)?;
        let strategy = match mode {
            FetchMode::Dynamic => self.dynamic_strategy.as_ref(),
            #[cfg(feature = "stealth")]
            FetchMode::Stealth => self.stealth_strategy.as_ref(),
            #[cfg(not(feature = "stealth"))]
            FetchMode::Stealth => None,
            _ => None,
        }
        .ok_or_else(|| WispError::Config(format!("{mode:?} mode requires browser strategy")))?;
        self.fetch_browser(req, strategy.as_ref()).await
    }

    #[cfg(not(feature = "browser"))]
    async fn fetch_browser_mode(&self, req: &Request, mode: FetchMode) -> Result<Response> {
        let _ = (req, mode);
        Err(WispError::Config(format!(
            "{mode:?} mode requires 'browser' feature"
        )))
    }

    #[cfg(feature = "browser")]
    pub(crate) async fn fetch_browser(
        &self,
        req: &Request,
        strategy: &dyn BrowserFetchStrategy,
    ) -> Result<Response> {
        let pool = self.browser_pool.as_ref().ok_or_else(|| {
            WispError::Browser(wisp_core::error::BrowserError::Other(
                "browser pool not configured (max_concurrent_pages=0)".into(),
            ))
        })?;
        let mut handle = pool.acquire().await?;

        if let Some(ref blocker) = self.config.domain_blocker
            && blocker.should_block(&req.url)
        {
            return Err(WispError::Config(format!(
                "domain blocked by DomainBlocker: {}",
                wisp_core::utils::sanitize_url(&req.url)
            )));
        }
        if let Some(ref blocker) = self.config.domain_blocker
            && let urls = blocker.blocked_domains()
            && !urls.is_empty()
        {
            handle
                .page_mut()
                .cmd(
                    "Network.setBlockedURLs",
                    serde_json::json!({ "urls": urls }),
                )
                .await?;
        }

        let work = strategy.fetch(handle.page_mut(), req);
        let result = tokio::time::timeout(Duration::from_secs(120), work)
            .await
            .map_err(|_| {
                WispError::Timeout(format!(
                    "fetch_browser 总超时（120s）: {}",
                    wisp_core::utils::sanitize_url(&req.url)
                ))
            })?;

        let _ = handle.page_mut().close().await;
        result
    }

    fn build_http_client(
        config: &FetchClientConfig,
        cookie_jar: Arc<wreq::cookie::Jar>,
    ) -> Result<Client> {
        // 所有 HttpConfig 字段由 http::Client 统一映射，避免手抄时丢字段（如 DoH）。
        let mut http = config.http.clone();
        http.cookie_jar = Some(cookie_jar);
        Client::from_config(http)
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
            headless: config.headless && !config.force_headed_offscreen,
            args: if config.force_headed_offscreen {
                vec![
                    "--window-position=-32000,-32000".to_string(),
                    "--window-size=1280,800".to_string(),
                ]
            } else {
                Vec::new()
            },
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
            drop(handle.spawn(async move {
                pool.shutdown().await;
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_client_preserves_full_config() {
        let config = FetchClientConfig {
            http: wisp_http::Config {
                dns_over_https: Some("https://1.1.1.1/dns-query".into()),
                ..Default::default()
            },
            max_concurrent_pages: 0,
            ..Default::default()
        };
        let client = FetchClient::new(config).expect("build client");
        let proxy = client
            .proxy_client("http://127.0.0.1:8080")
            .expect("build proxy client");
        assert_eq!(
            proxy.config_ref().proxy.as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(
            proxy.config_ref().dns_over_https.as_deref(),
            Some("https://1.1.1.1/dns-query")
        );
        let cached = client
            .proxy_client("http://127.0.0.1:8080")
            .expect("cached proxy client");
        assert!(Arc::ptr_eq(&proxy, &cached));
    }
}
