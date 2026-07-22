//! 缁熶竴 Fetcher 鍏ュ彛 - 鏍规嵁妯″紡鑷姩閫夋嫨搴曞眰瀹炵幇銆?
//!
//! # 涓夌妯″紡
//!
//! - `FetchMode::Http` - 蹇€?HTTP锛圱LS 鎸囩汗妯℃嫙锛屾绉掔骇锛屾棤娴忚鍣級
//! - `FetchMode::Dynamic` - 娴忚鍣ㄦ覆鏌擄紙JS 鎵ц锛岀绾э級
//! - `FetchMode::Stealth` - 闅愯韩娴忚鍣紙CF bypass + 浜虹被琛屼负妯℃嫙锛岀绾э級
//!
//! # 绀轰緥
//!
//! ```rust,no_run
//! use wisp::Fetcher;
//!
//! # async fn example() -> wisp::Result<()> {
//! // 涓夌妯″紡锛岀粺涓€ API
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

pub mod response;
pub mod session;

pub use response::{Response, Request, Method};
pub use session::Session;

use std::collections::HashMap;
use std::time::Duration;
use wreq_util::Profile;

use crate::error::Result;
use crate::http::block::DomainBlocker;

/// 鎶撳彇妯″紡銆?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchMode {
    /// 蹇€?HTTP锛圱LS 鎸囩汗妯℃嫙锛屾棤娴忚鍣級銆傛垚鏈渶浣庯紝姣绾с€?
    Http,
    /// 娴忚鍣ㄦ覆鏌擄紙JS 鎵ц锛屾棤 CF 缁曡繃锛夈€備腑绛夋垚鏈紝绉掔骇銆?
    Dynamic,
    /// 闅愯韩娴忚鍣紙CF bypass + 浜虹被琛屼负妯℃嫙锛夈€傛渶楂樻垚鏈紝绉掔骇銆?
    Stealth,
    /// 鑷姩妯″紡锛氬厛 HTTP锛岃鎷︽埅鍗囩骇 Stealth锛岄€夋嫨鍣ㄦ棤鍐呭鍗囩骇 Dynamic銆?
    /// 浠?Spider/Engine 鍦烘櫙鏀寔銆?
    Auto,
}

/// Fetcher 閰嶇疆銆?
#[derive(Debug, Clone)]
pub struct FetcherConfig {
    /// 璇锋眰瓒呮椂
    pub timeout: Duration,
    /// 浠ｇ悊 URL
    pub proxy: Option<String>,
    /// 娴忚鍣?headless 妯″紡
    pub headless: bool,
    /// TLS 鎸囩汗妯℃嫙锛圚ttp 妯″紡锛?
    pub emulation: Option<Profile>,
    /// 鑷畾涔?User-Agent
    pub user_agent: Option<String>,
    /// 鑷畾涔?headers
    pub headers: HashMap<String, String>,
    /// 鏈€澶ч噸瀹氬悜娆℃暟
    pub max_redirects: usize,
    /// 浜虹被琛屼负妯℃嫙锛圫tealth 妯″紡锛?
    pub human_mode: bool,
    /// CF 鎸戞垬瓒呮椂锛圫tealth 妯″紡锛?
    pub challenge_timeout: Duration,
    /// 绛夊緟鐗瑰畾 CSS 閫夋嫨鍣ㄥ嚭鐜?
    pub wait_for: Option<String>,
    /// 椤甸潰鍔犺浇鍚庨澶栫瓑寰咃紙姣锛?
    pub extra_wait_ms: u64,
    /// 鍩熷悕鎷︽埅鍣?
    pub domain_blocker: Option<DomainBlocker>,
    /// DNS-over-HTTPS
    pub dns_over_https: Option<String>,
}

impl Default for FetcherConfig {
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
        }
    }
}

/// 缁熶竴 Fetcher - 鏍规嵁妯″紡鑷姩閫夋嫨搴曞眰瀹炵幇銆?
pub struct Fetcher {
    mode: FetchMode,
    config: FetcherConfig,
}

impl Fetcher {
    /// 蹇€?HTTP 妯″紡锛圱LS 鎸囩汗锛屾绉掔骇锛夈€?
    pub fn http() -> FetcherBuilder {
        FetcherBuilder::new(FetchMode::Http)
    }

    /// 娴忚鍣ㄦ覆鏌撴ā寮忥紙JS 鎵ц锛岀绾э級銆?
    pub fn dynamic() -> FetcherBuilder {
        FetcherBuilder::new(FetchMode::Dynamic)
    }

    /// 闅愯韩妯″紡锛圕F bypass锛岀绾э級銆?
    pub fn stealth() -> FetcherBuilder {
        FetcherBuilder::new(FetchMode::Stealth)
    }

    /// 浠庡凡鏈夐厤缃垱寤?Fetcher銆?
    pub fn new(mode: FetchMode, config: FetcherConfig) -> Self {
        Self { mode, config }
    }

    /// 鑾峰彇褰撳墠妯″紡銆?
    pub fn mode(&self) -> FetchMode {
        self.mode
    }

    /// 鑾峰彇閰嶇疆寮曠敤銆?
    pub fn config(&self) -> &FetcherConfig {
        &self.config
    }

    /// GET 璇锋眰銆?
    pub async fn get(&self, url: &str) -> Result<Response> {
        self.fetch(Request::get(url)).await
    }

    /// POST 璇锋眰銆?
    pub async fn post(&self, url: &str, body: Option<&str>) -> Result<Response> {
        let mut req = Request::post(url, body.map(|b| b.to_string()));
        req.headers = self.config.headers.clone();
        self.fetch(req).await
    }

    /// 鍙戦€佽姹傦紙鏍规嵁妯″紡鍒嗗彂鍒颁笉鍚屽簳灞傚疄鐜帮級銆?
    pub async fn fetch(&self, req: Request) -> Result<Response> {
        match self.mode {
            FetchMode::Http => self.fetch_http(req).await,
            FetchMode::Dynamic => self.fetch_dynamic(req).await,
            FetchMode::Stealth => self.fetch_stealth(req).await,
            // Auto 妯″紡浠?Spider/Engine 鏀寔锛岀洿鎺ヨ皟鐢ㄦ椂鍥為€€鍒?Http
            FetchMode::Auto => self.fetch_http(req).await,
        }
    }

    // === Http 妯″紡锛歸req TLS 鎸囩汗妯℃嫙 ===
    async fn fetch_http(&self, req: Request) -> Result<Response> {
        let mut builder = crate::http::Client::builder()
            .timeout(self.config.timeout)
            .max_redirects(self.config.max_redirects);

        if let Some(ref proxy) = self.config.proxy {
            builder = builder.proxy(proxy);
        }
        if let Some(ref ua) = self.config.user_agent {
            builder = builder.user_agent(ua);
        }
        if let Some(emu) = self.config.emulation {
            builder = builder.emulation(emu);
        } else {
            builder = builder.no_emulation();
        }
        for (k, v) in &self.config.headers {
            builder = builder.header(k, v);
        }

        let client = builder.build()?;

        let resp = match req.method {
            Method::Get => client.get(&req.url).await?,
            Method::Post => client.post(&req.url, req.body.as_deref(), None).await?,
            Method::Put => client.put(&req.url, req.body.as_deref(), None).await?,
            Method::Delete => client.delete(&req.url).await?,
        };

        Ok(Response::from_http(
            resp.status,
            resp.url.clone(),
            resp.headers.clone(),
            resp.body.clone(),
            resp.headers.get("content-type").cloned().unwrap_or_default(),
            Some(req),
        ))
    }

    // === Dynamic 妯″紡锛氭祻瑙堝櫒娓叉煋 ===
    async fn fetch_dynamic(&self, req: Request) -> Result<Response> {
        self.fetch_browser_page(req, false).await
    }

    // === Stealth 妯″紡锛欳F bypass + 浜虹被琛屼负 ===
    async fn fetch_stealth(&self, req: Request) -> Result<Response> {
        self.fetch_browser_page(req, true).await
    }

    /// 娴忚鍣ㄦā寮忓叡鐢ㄩ€昏緫锛圖ynamic/Stealth 鍚堝苟锛夈€?
    /// `solve_cf=true` 鏃堕澶栨墽琛?CF 鎸戞垬瑙ｅ喅 + 浜虹被琛屼负妯℃嫙銆?
    async fn fetch_browser_page(&self, req: Request, solve_cf: bool) -> Result<Response> {
        let browser = self.launch_browser().await?;
        let page = browser.new_page().await?;

        page.goto(&req.url).await?;

        if solve_cf {
            // 妫€娴嬪苟瑙ｅ喅 Cloudflare 鎸戞垬
            let solver = crate::stealth::challenge::ChallengeSolver::new(&page);
            solver.solve(self.config.challenge_timeout).await?;

            // 浜虹被琛屼负妯℃嫙
            if self.config.human_mode {
                let human = crate::stealth::human::HumanBehavior::new(&page);
                human.random_delay(500, 1500).await?;
                human.random_scroll().await?;
                human.random_delay(300, 800).await?;
            }
        }

        // 绛夊緟鐗瑰畾閫夋嫨鍣?
        if let Some(ref selector) = self.config.wait_for {
            page.wait_for_selector(selector, self.config.timeout.as_millis() as u64).await?;
        }

        // 棰濆绛夊緟
        if self.config.extra_wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.config.extra_wait_ms)).await;
        }

        let result = self.extract_browser_response(&page, &req).await;
        browser.close().await?;
        result
    }

    // === 鍐呴儴杈呭姪 ===

    /// 鍚姩娴忚鍣紙Dynamic/Stealth 鍏辩敤锛夈€?
    async fn launch_browser(&self) -> Result<crate::browser::Browser> {
        let proxy_config = self.config.proxy.as_ref().map(|p| crate::config::ProxyConfig {
            server: p.clone(),
            username: None,
            password: None,
        });

        crate::browser::Browser::launch(crate::config::LaunchOptions {
            headless: self.config.headless,
            proxy: proxy_config,
            ..Default::default()
        }).await
    }

    /// 浠庢祻瑙堝櫒椤甸潰鎻愬彇缁熶竴 Response銆?
    async fn extract_browser_response(
        &self,
        page: &crate::browser::page::Page,
        req: &Request,
    ) -> Result<Response> {
        let html = page.evaluate_as_string("document.documentElement.outerHTML").await?;
        let title = page.evaluate_as_string("document.title").await?;
        let final_url = page.evaluate_as_string("window.location.href").await?;

        let cookies_raw = page.evaluate_as_string("document.cookie").await?;
        let cookies: Vec<String> = cookies_raw.split(';')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();

        // 鎺ㄦ柇鐘舵€佺爜
        let status = if html.contains("Access denied") || html.contains("403 Forbidden") {
            403
        } else if html.contains("Not Found") || html.contains("404") {
            404
        } else {
            200
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
}

/// Fetcher 鏋勫缓鍣?- 閾惧紡閰嶇疆銆?
pub struct FetcherBuilder {
    mode: FetchMode,
    config: FetcherConfig,
}

impl FetcherBuilder {
    pub(crate) fn new(mode: FetchMode) -> Self {
        Self { mode, config: FetcherConfig::default() }
    }

    /// 璁剧疆浠ｇ悊銆?
    pub fn proxy(mut self, url: &str) -> Self {
        self.config.proxy = Some(url.to_string());
        self
    }

    /// 璁剧疆瓒呮椂銆?
    pub fn timeout(mut self, d: Duration) -> Self {
        self.config.timeout = d;
        self
    }

    /// 璁剧疆 headless 妯″紡锛堟祻瑙堝櫒妯″紡锛夈€?
    pub fn headless(mut self, v: bool) -> Self {
        self.config.headless = v;
        self
    }

    /// 璁剧疆 TLS 鎸囩汗妯℃嫙锛圚ttp 妯″紡锛夈€?
    pub fn emulation(mut self, p: Profile) -> Self {
        self.config.emulation = Some(p);
        self
    }

    /// 鍏抽棴 TLS 鎸囩汗妯℃嫙銆?
    pub fn no_emulation(mut self) -> Self {
        self.config.emulation = None;
        self
    }

    /// 璁剧疆 User-Agent銆?
    pub fn user_agent(mut self, ua: &str) -> Self {
        self.config.user_agent = Some(ua.to_string());
        self
    }

    /// 娣诲姞鑷畾涔?header銆?
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.config.headers.insert(key.to_string(), value.to_string());
        self
    }

    /// 鍚敤/绂佺敤浜虹被琛屼负妯℃嫙锛圫tealth 妯″紡锛夈€?
    pub fn human_mode(mut self, v: bool) -> Self {
        self.config.human_mode = v;
        self
    }

    /// 璁剧疆 CF 鎸戞垬瓒呮椂锛圫tealth 妯″紡锛夈€?
    pub fn challenge_timeout(mut self, d: Duration) -> Self {
        self.config.challenge_timeout = d;
        self
    }

    /// 绛夊緟鐗瑰畾 CSS 閫夋嫨鍣ㄥ嚭鐜帮紙娴忚鍣ㄦā寮忥級銆?
    pub fn wait_for(mut self, selector: &str) -> Self {
        self.config.wait_for = Some(selector.to_string());
        self
    }

    /// 椤甸潰鍔犺浇鍚庨澶栫瓑寰咃紙姣锛夈€?
    pub fn extra_wait_ms(mut self, ms: u64) -> Self {
        self.config.extra_wait_ms = ms;
        self
    }

    /// 鍚敤鍐呯疆骞垮憡鎷︽埅銆?
    pub fn block_ads(mut self) -> Self {
        let mut blocker = self.config.domain_blocker.take().unwrap_or_default();
        blocker.enable_ad_blocking();
        self.config.domain_blocker = Some(blocker);
        self
    }

    /// 鎷︽埅鎸囧畾鍩熷悕銆?
    pub fn block_domains(mut self, domains: &[&str]) -> Self {
        let mut blocker = self.config.domain_blocker.take().unwrap_or_default();
        blocker.block_domains(domains);
        self.config.domain_blocker = Some(blocker);
        self
    }

    /// 璁剧疆 DNS-over-HTTPS銆?
    pub fn dns_over_https(mut self, url: &str) -> Self {
        self.config.dns_over_https = Some(url.to_string());
        self
    }

    /// 璁剧疆鏈€澶ч噸瀹氬悜娆℃暟銆?
    pub fn max_redirects(mut self, n: usize) -> Self {
        self.config.max_redirects = n;
        self
    }

    /// 鏋勫缓 Fetcher 瀹炰緥銆?
    pub fn build(self) -> Fetcher {
        Fetcher { mode: self.mode, config: self.config }
    }

    /// 蹇嵎鏂瑰紡锛歜uild + get銆?
    pub async fn get(self, url: &str) -> Result<Response> {
        self.build().get(url).await
    }

    /// 蹇嵎鏂瑰紡锛歜uild + post銆?
    pub async fn post(self, url: &str, body: Option<&str>) -> Result<Response> {
        self.build().post(url, body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            .timeout(Duration::from_secs(60))
            .emulation(Profile::Firefox128)
            .build();

        assert_eq!(fetcher.mode(), FetchMode::Http);
        assert_eq!(fetcher.config().proxy.as_deref(), Some("http://127.0.0.1:7897"));
        assert_eq!(fetcher.config().timeout, Duration::from_secs(60));
        assert_eq!(fetcher.config().emulation, Some(Profile::Firefox128));
    }

    #[test]
    fn test_fetcher_builder_stealth() {
        let fetcher = Fetcher::stealth()
            .headless(true)
            .human_mode(true)
            .challenge_timeout(Duration::from_secs(60))
            .proxy("http://127.0.0.1:7897")
            .build();

        assert_eq!(fetcher.mode(), FetchMode::Stealth);
        assert!(fetcher.config().headless);
        assert!(fetcher.config().human_mode);
        assert_eq!(fetcher.config().challenge_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_fetcher_builder_dynamic() {
        let fetcher = Fetcher::dynamic()
            .headless(false)
            .wait_for(".content")
            .extra_wait_ms(2000)
            .build();

        assert_eq!(fetcher.mode(), FetchMode::Dynamic);
        assert!(!fetcher.config().headless);
        assert_eq!(fetcher.config().wait_for.as_deref(), Some(".content"));
        assert_eq!(fetcher.config().extra_wait_ms, 2000);
    }

    #[test]
    fn test_fetcher_builder_block_ads() {
        let fetcher = Fetcher::dynamic()
            .block_ads()
            .block_domains(&["analytics.example.com"])
            .build();

        let blocker = fetcher.config().domain_blocker.as_ref().unwrap();
        assert!(blocker.is_ad_blocking_enabled());
        assert!(blocker.should_block("https://analytics.example.com/track"));
    }

    #[test]
    fn test_fetcher_config_default() {
        let config = FetcherConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert!(config.headless);
        assert!(config.human_mode);
        assert_eq!(config.emulation, Some(Profile::Chrome136));
        assert!(config.proxy.is_none());
        assert!(config.domain_blocker.is_none());
    }
}
