//! 统一请求客户端 — 封装 HTTP Client 和 BrowserPool。
//!
//! - HTTP 请求：共享 `http::Client`（连接池复用）
//! - 浏览器请求：通过 `BrowserPool`（实例复用，RAII 自动归还）

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use wreq_util::Profile;

use crate::browser::BrowserPool;
use crate::config::LaunchOptions;
use crate::error::{Result, WispError};
use crate::http::{block::DomainBlocker, Client};
use crate::stealth::challenge::ChallengeSolver;
use crate::stealth::human::HumanBehavior;

use super::response::{Method, Request, Response};

/// 统一请求客户端配置。
#[derive(Debug, Clone)]
pub struct FetchClientConfig {
    /// 请求超时
    pub timeout: Duration,
    /// 代理 URL
    pub proxy: Option<String>,
    /// 浏览器 headless 模式
    pub headless: bool,
    /// TLS 指纹模拟（Http 模式）
    pub emulation: Option<Profile>,
    /// 自定义 User-Agent
    pub user_agent: Option<String>,
    /// 自定义 headers
    pub headers: HashMap<String, String>,
    /// 最大重定向次数
    pub max_redirects: usize,
    /// 人类行为模拟（Stealth 模式）
    pub human_mode: bool,
    /// CF 挑战超时（Stealth 模式）
    pub challenge_timeout: Duration,
    /// 等待特定 CSS 选择器出现
    pub wait_for: Option<String>,
    /// 页面加载后额外等待（毫秒）
    pub extra_wait_ms: u64,
    /// 域名拦截器
    pub domain_blocker: Option<DomainBlocker>,
    /// DNS-over-HTTPS
    pub dns_over_https: Option<String>,
    /// BrowserPool 最大实例数（0 = 禁用池化）
    pub browser_pool_size: usize,
    /// BrowserPool 空闲超时
    pub browser_idle_timeout: Duration,
}

impl Default for FetchClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            proxy: None,
            headless: true,
            emulation: Some(Profile::Chrome136),
            user_agent: None,
            headers: HashMap::new(),
            max_redirects: 10,
            human_mode: true,
            challenge_timeout: Duration::from_secs(30),
            wait_for: None,
            extra_wait_ms: 0,
            domain_blocker: None,
            dns_over_https: None,
            browser_pool_size: 2,
            browser_idle_timeout: Duration::from_secs(300),
        }
    }
}

/// 统一请求客户端：封装 HTTP Client 和 BrowserPool。
///
/// - HTTP 请求：共享 `http::Client`（连接池复用）
/// - 浏览器请求：通过 `BrowserPool`（实例复用，RAII 自动归还）
pub struct FetchClient {
    http: Arc<Client>,
    browser_pool: Option<Arc<BrowserPool>>,
    config: FetchClientConfig,
}

impl FetchClient {
    /// 创建 FetchClient。
    pub fn new(config: FetchClientConfig) -> Result<Self> {
        let http = Arc::new(Self::build_http_client(&config)?);
        let browser_pool = Self::build_browser_pool(&config);
        Ok(Self {
            http,
            browser_pool,
            config,
        })
    }

    /// 获取 HTTP 客户端引用。
    pub fn http(&self) -> &Client {
        &self.http
    }

    /// 获取浏览器池引用（若有）。
    pub fn browser_pool(&self) -> Option<&Arc<BrowserPool>> {
        self.browser_pool.as_ref()
    }

    /// 获取配置引用。
    pub fn config(&self) -> &FetchClientConfig {
        &self.config
    }

    /// HTTP 请求（共享 Client，连接复用）。
    pub async fn fetch_http(&self, req: &Request) -> Result<Response> {
        let extra_headers: Vec<(String, String)> = req
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let resp = match req.method {
            Method::Get => self.http.get(&req.url, &extra_headers).await?,
            Method::Post => {
                self.http
                    .post(&req.url, req.body.as_deref(), None, &extra_headers)
                    .await?
            }
            Method::Put => {
                self.http
                    .put(&req.url, req.body.as_deref(), None, &extra_headers)
                    .await?
            }
            Method::Delete => self.http.delete(&req.url, &extra_headers).await?,
        };
        Ok(Response::from_http(
            resp.status,
            resp.url.clone(),
            resp.headers.clone(),
            resp.body.clone(),
            resp.headers
                .get("content-type")
                .cloned()
                .unwrap_or_default(),
            Some(req.clone()),
        ))
    }

    /// 浏览器请求（通过 BrowserPool，实例复用）。
    /// `solve_cf=true` 时执行 CF 挑战解决 + 人类行为模拟。
    pub async fn fetch_browser(&self, req: &Request, solve_cf: bool) -> Result<Response> {
        let pool = self.browser_pool.as_ref().ok_or_else(|| {
            WispError::CdpError("browser pool not configured (pool_size=0)".into())
        })?;
        // RAII: handle Drop 时自动归还到池，无论成功失败
        let handle = pool.acquire().await?;
        let result = self.do_browser_work(&handle, req, solve_cf).await;
        // handle 在此处 Drop，自动归还
        result
    }

    /// 浏览器工作逻辑（Dynamic/Stealth 共用）。
    async fn do_browser_work(
        &self,
        handle: &crate::browser::pool::BrowserHandle,
        req: &Request,
        solve_cf: bool,
    ) -> Result<Response> {
        let mut page = handle.new_page().await?;

        // 用 inner 提取实际工作，确保无论成功/失败都显式关闭 tab（避免泄漏）
        let result = self.do_browser_work_inner(&mut page, req, solve_cf).await;

        // 无论 result 是 Ok 或 Err，都关闭 tab
        let _ = page.close().await;

        result
    }

    async fn do_browser_work_inner(
        &self,
        page: &mut crate::browser::page::Page,
        req: &Request,
        solve_cf: bool,
    ) -> Result<Response> {
        // 启用 Network 域以捕获真实 HTTP 状态码
        let _ = page.cmd("Network.enable", serde_json::json!({})).await;

        page.goto(&req.url).await?;

        // 通过 CDP Network.responseReceived 获取真实状态码
        let nav_status = self.capture_navigation_status(page).await;

        if solve_cf {
            // 检测并解决 Cloudflare 挑战
            let solver = ChallengeSolver::new(page);
            solver.solve(self.config.challenge_timeout).await?;

            // 人类行为模拟
            if self.config.human_mode {
                let human = HumanBehavior::new(page);
                human.random_delay(500, 1500).await?;
                human.random_scroll().await?;
                human.random_delay(300, 800).await?;
            }
        }

        // 等待特定选择器
        if let Some(ref selector) = self.config.wait_for {
            page.wait_for_selector(selector, self.config.timeout.as_millis() as u64)
                .await?;
        }

        // 额外等待
        if self.config.extra_wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.config.extra_wait_ms)).await;
        }

        self.extract_browser_response(page, req, nav_status).await
    }

    /// 从 CDP Network.responseReceived 事件中捕获导航请求的真实 HTTP 状态码。
    async fn capture_navigation_status(&self, page: &crate::browser::page::Page) -> Option<u16> {
        let sid = page.session_id.clone();
        let event = page
            .session
            .wait_for_event(
                move |e| {
                    if e.method == "Network.responseReceived" {
                        let is_doc =
                            e.params.get("type").and_then(|t| t.as_str()) == Some("Document");
                        let match_session =
                            e.session_id.as_deref() == Some(sid.as_str()) || e.session_id.is_none();
                        is_doc && match_session
                    } else {
                        false
                    }
                },
                5000,
            )
            .await;

        event.ok().and_then(|e| {
            e.params
                .get("response")
                .and_then(|r| r.get("status"))
                .and_then(|s| s.as_u64())
                .map(|s| s as u16)
        })
    }

    /// 从浏览器页面提取统一 Response。
    async fn extract_browser_response(
        &self,
        page: &crate::browser::page::Page,
        req: &Request,
        nav_status: Option<u16>,
    ) -> Result<Response> {
        let html = page
            .evaluate_as_string("document.documentElement.outerHTML")
            .await?;
        let title = page.evaluate_as_string("document.title").await?;
        let final_url = page.evaluate_as_string("window.location.href").await?;

        let cookies_raw = page.evaluate_as_string("document.cookie").await?;
        let cookies: Vec<String> = cookies_raw
            .split(';')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();

        // 状态码：优先用 CDP 真实值，fallback 到 <title> 精确匹配
        let status = if let Some(code) = nav_status {
            code
        } else {
            let title_lower = title.to_lowercase();
            if title_lower.contains("403") && title_lower.contains("forbidden") {
                403
            } else if title_lower.contains("404") && title_lower.contains("not found") {
                404
            } else {
                200
            }
        };

        Ok(Response::from_browser(
            status,
            final_url,
            html,
            title,
            cookies,
            Some(req.clone()),
        ))
    }

    fn build_http_client(config: &FetchClientConfig) -> Result<Client> {
        let mut builder = Client::builder()
            .timeout(config.timeout)
            .max_redirects(config.max_redirects);

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

    fn build_browser_pool(config: &FetchClientConfig) -> Option<Arc<BrowserPool>> {
        if config.browser_pool_size == 0 {
            return None;
        }
        let proxy_config = config.proxy.as_ref().map(|p| crate::config::ProxyConfig {
            server: p.clone(),
            username: None,
            password: None,
        });
        let launch_options = LaunchOptions {
            headless: config.headless,
            proxy: proxy_config,
            ..Default::default()
        };
        Some(BrowserPool::new(
            config.browser_pool_size,
            config.browser_idle_timeout,
            launch_options,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_client_config_default() {
        let config = FetchClientConfig::default();
        assert_eq!(config.browser_pool_size, 2);
        assert!(config.headless);
        assert!(config.human_mode);
    }

    #[test]
    fn test_fetch_client_http_only() {
        // browser_pool_size=0 → 无浏览器池
        let config = FetchClientConfig {
            browser_pool_size: 0,
            ..Default::default()
        };
        let client = FetchClient::new(config).expect("build client");
        assert!(client.browser_pool().is_none());
        assert!(client.http().config_ref().timeout == Duration::from_secs(30));
    }

    #[test]
    fn test_fetch_client_with_browser_pool() {
        let config = FetchClientConfig::default();
        let client = FetchClient::new(config).expect("build client");
        assert!(client.browser_pool().is_some());
    }
}
