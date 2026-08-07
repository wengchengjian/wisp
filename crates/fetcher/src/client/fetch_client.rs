//! FetchClient 实现：封装 HTTP Client 与 BrowserPool。

use super::config::FetchClientConfig;
use moka::sync::Cache;
#[cfg(feature = "browser")]
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "browser")]
use std::time::Duration;

#[cfg(feature = "stealth")]
use crate::cookie::CfCookieJar;
use crate::cookie::CompositeCookieJar;
use crate::cookie::{CookieJar, HttpCookieJar};
#[cfg(feature = "stealth")]
use crate::strategy::StealthStrategy;
#[cfg(feature = "browser")]
use crate::strategy::{BrowserFetchStrategy, DynamicStrategy};
#[cfg(feature = "browser")]
use wisp_browser::BrowserPool;
#[cfg(feature = "browser")]
use wisp_core::config::LaunchOptions;
use wisp_core::error::{Result, WispError};
use wisp_core::{FetchMode, Request, Response};
use wisp_http::Client;
use wreq_util::Profile;

/// 单次 Fetch 的传输选项；`Default` 表示无额外选项。
#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    /// HTTP TLS 指纹模拟（仅 HTTP 模式生效）。
    pub emulation: Option<Profile>,
}

/// 统一请求客户端：封装 HTTP Client 和 BrowserPool。
///
/// 对外深 seam 是 [`FetchClient::fetch`] / [`FetchClient::fetch_with`]；per-request proxy 与 cookie 状态都在内部管理。
///
/// 浏览器策略通过注册表管理（替代固定字段）：`new` 时注册 `Dynamic`/`Stealth` 默认策略，
/// 可通过 [`FetchClient::with_strategy`] 注入自定义策略（如测试 mock 或新策略实现）。
pub struct FetchClient {
    http: Arc<Client>,
    http_jar: Arc<HttpCookieJar>,
    proxy_clients: Cache<String, Arc<Client>>,
    #[cfg(feature = "browser")]
    browser_pool: Option<Arc<BrowserPool>>,
    #[cfg(feature = "browser")]
    browser_strategies: HashMap<FetchMode, Arc<dyn BrowserFetchStrategy>>,
    config: FetchClientConfig,
    cookie_jar: Arc<dyn CookieJar>,
    /// CF 快速路径专用的共享 HTTP 客户端（Chrome 指纹 + 全局代理），懒加载。
    /// 与每次重建相比，可复用连接、减少握手，更贴近真实浏览器行为。
    emulated_http: std::sync::OnceLock<Arc<Client>>,
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
        let browser_strategies = {
            let mut map: HashMap<FetchMode, Arc<dyn BrowserFetchStrategy>> = HashMap::new();
            map.insert(
                FetchMode::Dynamic,
                Arc::new(DynamicStrategy::from_config(&config)),
            );
            #[cfg(feature = "stealth")]
            map.insert(
                FetchMode::Stealth,
                Arc::new(StealthStrategy::from_config(
                    &config.stealth_config(),
                    cookie_jar.clone(),
                )),
            );
            map
        };
        Ok(Self {
            http,
            http_jar,
            proxy_clients,
            #[cfg(feature = "browser")]
            browser_pool,
            #[cfg(feature = "browser")]
            browser_strategies,
            config,
            cookie_jar,
            emulated_http: std::sync::OnceLock::new(),
        })
    }

    /// 注入或覆盖某模式的浏览器策略（builder 风格）。
    ///
    /// 用于测试 mock 或扩展新策略（如 Playwright）。仅 browser feature 下生效；
    /// 无 browser feature 时静默返回 `self`（无策略可注册）。
    #[cfg(feature = "browser")]
    #[must_use]
    pub fn with_strategy(
        mut self,
        mode: FetchMode,
        strategy: Arc<dyn BrowserFetchStrategy>,
    ) -> Self {
        self.browser_strategies.insert(mode, strategy);
        self
    }

    /// 获取配置引用。
    #[must_use]
    pub fn config(&self) -> &FetchClientConfig {
        &self.config
    }

    /// 按模式分发请求；Auto 属于 crawl Engine，不由 FetchClient 处理。
    pub async fn fetch(&self, req: &Request, mode: FetchMode) -> Result<Response> {
        self.fetch_with(req, mode, &FetchOptions::default()).await
    }

    /// 按模式分发请求并携带单次传输选项；Auto 属于 crawl Engine，不由 FetchClient 处理。
    pub async fn fetch_with(
        &self,
        req: &Request,
        mode: FetchMode,
        options: &FetchOptions,
    ) -> Result<Response> {
        match mode {
            FetchMode::Http => {
                if let Some(emulation) = options.emulation {
                    self.fetch_http_with_emulation(req, emulation).await
                } else {
                    self.fetch_http(req).await
                }
            }
            FetchMode::Auto => Err(WispError::Config(
                "Auto mode is owned by wisp_crawl Engine; use Http/Dynamic/Stealth explicitly"
                    .into(),
            )),
            FetchMode::Dynamic | FetchMode::Stealth => self.fetch_browser_mode(req, mode).await,
        }
    }

    /// Auto 快速路径：若共享 cookie seam 已有会话 cookie，则走 HTTP。
    #[doc(hidden)]
    pub async fn try_http_with_session_cookie(&self, req: &Request) -> Result<Option<Response>> {
        let Some(url_parsed) = url::Url::parse(&req.url).ok() else {
            tracing::debug!(
                "try_http_with_session_cookie: URL 解析失败, url={}",
                req.url
            );
            return Ok(None);
        };
        let Some(cookie_header) = self.cookie_jar.header(&url_parsed).await else {
            tracing::debug!(
                "try_http_with_session_cookie: cookie jar 无 cookie, url={}, host={:?}",
                req.url,
                url_parsed.host_str()
            );
            return Ok(None);
        };
        let mut http_req = req.clone();
        http_req
            .headers
            .insert("Cookie".to_string(), cookie_header.clone());
        if let Some(ua) = self.cookie_jar.ua(&url_parsed).await {
            http_req
                .headers
                .insert("User-Agent".to_string(), ua.clone());
            // 对齐 sec-ch-ua 相关头：wreq 内置档位默认平台可能是 macOS，与真实浏览器
            // Windows 不一致。cf_clearance 绑定签发时的 UA/session，回放时若 sec-ch-ua 与
            // UA 不匹配，CF 会判定会话无效而 403。故按 cookie_jar 保存的真实浏览器 UA
            // 动态对齐（3-brand、Google Chrome 在前，与真实 Chrome 119+ 一致）。
            if let Some((schua, platform, mobile)) = chrome_sec_ch_ua(&ua) {
                http_req.headers.insert("sec-ch-ua".to_string(), schua);
                http_req
                    .headers
                    .insert("sec-ch-ua-platform".to_string(), platform);
                http_req
                    .headers
                    .insert("sec-ch-ua-mobile".to_string(), mobile);
            }
        }
        // 补全浏览器导航特征头，尽量贴近签发 cf_clearance 时的浏览器请求，
        // 减少 CF 因「非浏览器请求」判定 cf_clearance 无效而 403。
        http_req
            .headers
            .entry("Sec-Fetch-Site".to_string())
            .or_insert_with(|| "same-origin".to_string());
        http_req
            .headers
            .entry("Sec-Fetch-Mode".to_string())
            .or_insert_with(|| "navigate".to_string());
        http_req
            .headers
            .entry("Sec-Fetch-User".to_string())
            .or_insert_with(|| "?1".to_string());
        http_req
            .headers
            .entry("Sec-Fetch-Dest".to_string())
            .or_insert_with(|| "document".to_string());
        http_req
            .headers
            .entry("Upgrade-Insecure-Requests".to_string())
            .or_insert_with(|| "1".to_string());
        // 使用与浏览器尽可能接近的 TLS 指纹（自动对齐浏览器版本，见 select_cf_profile）。
        let resp = self.cf_http_client()?.fetch(&http_req).await?;
        let ua = http_req
            .headers
            .get("User-Agent")
            .cloned()
            .unwrap_or_default();
        tracing::info!(
            "try_http_with_session_cookie: url={} status={} cookie={} ua={}",
            req.url,
            resp.status,
            cookie_header,
            ua
        );
        if resp.status == 200 {
            // 复用成功 → 续期 CF 会话 TTL。否则 jar 的 30 分钟 TTL 会先于
            // cookie 真实有效期过期，导致 UA/sec-ch-ua 对齐信息丢失（ua() 只读
            // cf jar），后续快速路径带 cookie 却无对齐头 → CF 403 → 无谓回退 Stealth。
            self.cookie_jar.touch(&url_parsed).await;
            Ok(Some(resp))
        } else {
            Ok(None)
        }
    }

    pub(crate) async fn fetch_http(&self, req: &Request) -> Result<Response> {
        self.ensure_domain_allowed(req)?;
        // 请求级代理优先，否则回退到全局配置代理。这样 HTTP 与浏览器走同一出口 IP，
        // 才能复用浏览器签发的 CF 会话 cookie（cf_clearance 绑定签发时的 IP）。
        let proxy = req.proxy.clone().or_else(|| self.config.proxy.clone());
        if let Some(proxy) = proxy {
            let client = self.proxy_client(&proxy)?;
            return client.fetch(req).await;
        }
        self.http.fetch(req).await
    }

    fn ensure_domain_allowed(&self, req: &Request) -> Result<()> {
        if let Some(ref blocker) = self.config.domain_blocker
            && blocker.should_block(&req.url)
        {
            return Err(WispError::Config(format!(
                "domain blocked by DomainBlocker: {}",
                wisp_core::utils::sanitize_url(&req.url)
            )));
        }
        Ok(())
    }

    /// 使用共享 cookie/代理配置和指定 TLS 指纹执行 HTTP 抓取。
    ///
    /// MCP `fetch_page` 的 per-call emulation 需要独立 Client，因为 wreq 的
    /// TLS 指纹属于客户端级配置；共享 cookie jar 和 HTTP 配置仍从本客户端读取。
    async fn fetch_http_with_emulation(
        &self,
        req: &Request,
        emulation: Profile,
    ) -> Result<Response> {
        self.ensure_domain_allowed(req)?;
        let mut http = self.config.http.clone();
        http.emulation = Some(emulation);
        http.cookie_jar = Some(self.http_jar.jar());
        // 请求级代理优先，否则回退全局配置代理（与 fetch_http 保持一致出口 IP）。
        if let Some(proxy) = req.proxy.clone().or_else(|| self.config.proxy.clone()) {
            http.proxy = Some(proxy);
        }
        Client::from_config(http)?.fetch(req).await
    }

    /// CF 快速路径共享 HTTP 客户端：Chrome 指纹（自动对齐浏览器版本）+ 全局代理，
    /// 懒加载并复用，避免每次重建连接。
    fn cf_http_client(&self) -> Result<Arc<Client>> {
        if let Some(client) = self.emulated_http.get() {
            return Ok(client.clone());
        }
        let mut http = self.config.http.clone();
        http.emulation = Some(self.select_cf_profile());
        http.cookie_jar = Some(self.http_jar.jar());
        if let Some(proxy) = self.config.proxy.clone() {
            http.proxy = Some(proxy);
        }
        let client = Arc::new(Client::from_config(http)?);
        let _ = self.emulated_http.set(client.clone());
        Ok(client)
    }

    /// 选择 CF 快速路径的 TLS 指纹档位。
    ///
    /// 固定使用 `Chrome148`：wreq-util 该档位与 CF 复用实测兼容（携带同一
    /// cf_clearance 走 HTTP 返回 200），而 `Chrome149` 档位返回 403（不兼容）。
    ///
    /// 不做动态探测选档：浏览器版本由安装器固定为 Chrome 148，指纹档位与之一致，
    /// 避免浏览器版本漂移导致的指纹不自洽与排查困难。
    fn select_cf_profile(&self) -> Profile {
        Profile::Chrome148
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
        let strategy = self
            .browser_strategies
            .get(&mode)
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
        self.ensure_domain_allowed(req)?;
        let pool = self.browser_pool.as_ref().ok_or_else(|| {
            WispError::Browser(wisp_core::error::BrowserError::Other(
                "browser pool not configured (max_concurrent_pages=0)".into(),
            ))
        })?;
        // 浏览器资源生命周期（acquire/close/超时/blockedURLs）已下沉到 BrowserPool::fetch_with_strategy。
        let blocked_urls = self
            .config
            .domain_blocker
            .as_ref()
            .map(|blocker| blocker.blocked_domains())
            .unwrap_or_default();
        pool.fetch_with_strategy(strategy, req, &blocked_urls, Duration::from_secs(120))
            .await
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
                // 临时诊断：窗口显示在屏幕可见区域，便于观察 Turnstile 交互。
                // 生产环境应改回 `--window-position=-32000,-32000`（offscreen）。
                vec![
                    "--window-position=300,100".to_string(),
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

/// 从浏览器 UA 解析 Chrome 主版本与平台，构造与 UA 一致的 sec-ch-ua 头。
///
/// `cf_clearance` 绑定签发时的 UA，回放时若 sec-ch-ua 与 UA 不匹配，CF 会判定会话无效。
/// 真实 Chrome 119+ 的 sec-ch-ua 是 3-brand 且 `"Google Chrome"` 在前（如
/// `"Google Chrome";v="149", "Chromium";v="149", "Not)A;Brand";v="24"`），
/// 与 wreq 内置 Chrome149 档一致。旧版 wisp 曾误用 2-brand（`Chromium` 在前），
/// 与真实浏览器不符导致 HTTP 复用 403，此处统一修正为 3-brand。
///
/// 返回 `(sec-ch-ua, sec-ch-ua-platform, sec-ch-ua-mobile)`；无法解析版本时返回 `None`。
fn chrome_sec_ch_ua(ua: &str) -> Option<(String, String, String)> {
    let major = ua.split("Chrome/").nth(1)?.split('.').next()?;
    if major.is_empty() {
        return None;
    }
    let platform = if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Mac OS X") {
        "macOS"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("iPhone") {
        "iOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        "Windows"
    };
    Some((
        format!(r#""Google Chrome";v="{major}", "Chromium";v="{major}", "Not)A;Brand";v="24""#),
        format!("\"{platform}\""),
        "?0".to_string(),
    ))
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
    fn ensure_domain_allowed_blocks_configured_domain() {
        use wisp_http::DomainBlocker;
        let mut blocker = DomainBlocker::new();
        blocker.block_domain("ads.example.com");
        let config = FetchClientConfig {
            domain_blocker: Some(blocker),
            max_concurrent_pages: 0,
            ..Default::default()
        };
        let client = FetchClient::new(config).expect("build client");
        assert!(
            client
                .ensure_domain_allowed(&Request::get("https://ads.example.com/ad.js"))
                .is_err()
        );
        assert!(
            client
                .ensure_domain_allowed(&Request::get("https://ok.example.com/"))
                .is_ok()
        );
    }
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

    /// CF 快速路径指纹固定为 Chrome148，与浏览器安装器固定的版本一致。
    #[test]
    fn select_cf_profile_is_fixed_chrome148() {
        let config = FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        };
        let client = FetchClient::new(config).expect("build client");
        assert_eq!(
            client.select_cf_profile(),
            Profile::Chrome148,
            "CF 快速路径应固定 Chrome148，不做动态选档"
        );
    }

    #[test]
    fn chrome_sec_ch_ua_matches_browser_ua() {
        // 真实 Chrome 149 Windows：3-brand（Google Chrome 在前）、平台 Windows、非移动。
        let (schua, platform, mobile) = chrome_sec_ch_ua(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.7827.155 Safari/537.36",
        )
        .expect("应能解析 Windows UA");
        assert_eq!(
            schua,
            r#""Google Chrome";v="149", "Chromium";v="149", "Not)A;Brand";v="24""#
        );
        assert_eq!(platform, "\"Windows\"");
        assert_eq!(mobile, "?0");

        // macOS 平台识别。
        let (_, platform, _) = chrome_sec_ch_ua(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36",
        )
        .expect("应能解析 macOS UA");
        assert_eq!(platform, "\"macOS\"");

        // 版本号取自 UA 中的主版本。
        let (schua, _, _) =
            chrome_sec_ch_ua("Mozilla/5.0 ... Chrome/120.0.1 Safari/537.36").expect("应能解析版本");
        assert_eq!(
            schua,
            r#""Google Chrome";v="120", "Chromium";v="120", "Not)A;Brand";v="24""#
        );

        // 非 Chrome UA 无法解析 → None。
        assert!(chrome_sec_ch_ua("curl/8.0").is_none());
        assert!(chrome_sec_ch_ua("").is_none());
    }

    /// 集成测试：浏览器启动成功 → 固定指纹档位不变（Chrome148）。
    ///
    /// 指纹档位固定为 Chrome148，不随浏览器版本变化，保证 CF 复用稳定。
    /// 需要真实 Chrome 环境，默认忽略。
    #[cfg(feature = "browser")]
    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn test_select_cf_profile_fixed_despite_browser_version() {
        let config = FetchClientConfig {
            max_concurrent_pages: 4,
            ..Default::default()
        };
        let client = FetchClient::new(config).expect("build client");

        // 核心断言：无论浏览器版本如何，指纹档位恒为 Chrome148。
        assert_eq!(
            client.select_cf_profile(),
            Profile::Chrome148,
            "指纹档位固定 Chrome148，不随浏览器版本变化"
        );
    }
}
