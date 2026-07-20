//! High-level Scraper API with automatic Cloudflare bypass.
//!
//! Combines browser automation, challenge solving, proxy rotation,
//! and human behavior simulation into a simple interface.

use std::collections::HashMap;
use std::time::Duration;

use crate::browser::Browser;
use crate::challenge::ChallengeSolver;
use crate::config::{LaunchOptions, ProxyConfig};
use crate::error::{WispError, Result};
use crate::human::HumanBehavior;
use crate::proxy::{ProxyPool, RotationStrategy};

/// Response from a scrape operation.
#[derive(Debug, Clone)]
pub struct ScrapeResponse {
    /// HTTP status code (200 if page loaded, 403 if blocked, etc.)
    pub status: u16,
    /// Final URL after redirects.
    pub url: String,
    /// Full page HTML.
    pub html: String,
    /// Page title.
    pub title: String,
    /// Cookies from the page (as "name=value" strings).
    pub cookies: Vec<String>,
}

impl ScrapeResponse {
    /// Parse the HTML content into a queryable Node.
    pub fn parse(&self) -> crate::parser::Node {
        crate::parser::Node::from_html(&self.html)
    }
}

/// Options for a scrape request.
#[derive(Debug, Clone)]
pub struct RequestOptions {
    /// Wait for a specific CSS selector to appear before extracting content.
    pub wait_for: Option<String>,
    /// Maximum time to wait for the page + challenge.
    pub timeout: Duration,
    /// Extra wait after page load (ms).
    pub extra_wait_ms: u64,
    /// Custom headers to set on the page.
    pub headers: HashMap<String, String>,
    /// Cookies to set before navigation.
    pub cookies: Vec<String>,
}

impl Default for RequestOptions {
    fn default() -> Self {
        Self {
            wait_for: None,
            timeout: Duration::from_secs(60),
            extra_wait_ms: 1000,
            headers: HashMap::new(),
            cookies: Vec::new(),
        }
    }
}

/// Builder for configuring a Scraper instance.
pub struct ScraperBuilder {
    headless: bool,
    proxies: Vec<String>,
    proxy_strategy: RotationStrategy,
    human_mode: bool,
    challenge_timeout: Duration,
    max_retries: u32,
}

impl ScraperBuilder {
    pub fn new() -> Self {
        Self {
            headless: true,
            proxies: Vec::new(),
            proxy_strategy: RotationStrategy::Sequential,
            human_mode: true,
            challenge_timeout: Duration::from_secs(30),
            max_retries: 3,
        }
    }

    /// Run browser in headless mode (default: true).
    pub fn headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    /// Set proxy list.
    pub fn proxies(mut self, proxies: Vec<String>) -> Self {
        self.proxies = proxies;
        self
    }

    /// Set proxy rotation strategy.
    pub fn proxy_strategy(mut self, strategy: RotationStrategy) -> Self {
        self.proxy_strategy = strategy;
        self
    }

    /// Enable human behavior simulation (default: true).
    pub fn human_mode(mut self, enabled: bool) -> Self {
        self.human_mode = enabled;
        self
    }

    /// Timeout for Cloudflare challenge solving.
    pub fn challenge_timeout(mut self, timeout: Duration) -> Self {
        self.challenge_timeout = timeout;
        self
    }

    /// Maximum retries with different proxies on failure.
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Build the Scraper.
    pub fn build(self) -> Result<Scraper> {
        let proxy_pool = if self.proxies.is_empty() {
            None
        } else {
            Some(ProxyPool::new(self.proxies, self.proxy_strategy))
        };

        Ok(Scraper {
            proxy_pool,
            headless: self.headless,
            challenge_timeout: self.challenge_timeout,
            max_retries: self.max_retries,
            human_mode: self.human_mode,
        })
    }
}

/// High-level scraper with automatic Cloudflare bypass.
pub struct Scraper {
    proxy_pool: Option<ProxyPool>,
    headless: bool,
    challenge_timeout: Duration,
    max_retries: u32,
    human_mode: bool,
}

impl Scraper {
    /// Create a new ScraperBuilder.
    pub fn builder() -> ScraperBuilder {
        ScraperBuilder::new()
    }

    /// GET a URL with automatic Cloudflare challenge bypass.
    ///
    /// Launches a browser, navigates to the URL, detects and solves any
    /// Cloudflare challenge, optionally simulates human behavior, and
    /// returns the page content.
    pub async fn get(&self, url: &str) -> Result<ScrapeResponse> {
        self.get_with_options(url, RequestOptions::default()).await
    }

    /// GET with custom options.
    pub async fn get_with_options(&self, url: &str, opts: RequestOptions) -> Result<ScrapeResponse> {
        let mut last_err = None;
        let attempts = if self.proxy_pool.is_some() { self.max_retries } else { 1 };

        for _ in 0..attempts {
            match self.try_get(url, &opts).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    tracing::warn!("Scrape attempt failed: {e}");
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| WispError::CdpError("all attempts failed".into())))
    }

    /// Get text content of a specific element.
    pub async fn get_text(&self, url: &str, selector: &str) -> Result<String> {
        let opts = RequestOptions {
            wait_for: Some(selector.to_string()),
            ..Default::default()
        };

        let proxy = self.proxy_pool.as_ref().and_then(|p| p.next());
        let browser = self.launch_browser(proxy.as_deref()).await?;
        let page = browser.new_page().await?;

        page.goto(url).await?;

        // Solve Cloudflare challenge if present
        let solver = ChallengeSolver::new(&page);
        solver.solve(self.challenge_timeout).await?;

        // Wait for target element
        page.wait_for_selector(selector, 10000).await?;
        let text = page.text_content(selector).await?;

        browser.close().await?;
        Ok(text)
    }

    /// Internal: single attempt to get a URL.
    async fn try_get(&self, url: &str, opts: &RequestOptions) -> Result<ScrapeResponse> {
        let proxy = self.proxy_pool.as_ref().and_then(|p| p.next());
        let browser = self.launch_browser(proxy.as_deref()).await?;
        let page = browser.new_page().await?;

        // Navigate
        page.goto(url).await?;

        // Detect and solve Cloudflare challenge
        let solver = ChallengeSolver::new(&page);
        solver.solve(self.challenge_timeout).await?;

        // Human behavior simulation
        if self.human_mode {
            let human = HumanBehavior::new(&page);
            human.random_delay(500, 1500).await?;
            human.random_scroll().await?;
            human.random_delay(300, 800).await?;
        }

        // Wait for specific element if requested
        if let Some(ref selector) = opts.wait_for {
            page.wait_for_selector(selector, opts.timeout.as_millis() as u64).await?;
        }

        // Extra wait
        if opts.extra_wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(opts.extra_wait_ms)).await;
        }

        // Extract content
        let html = page.evaluate_as_string("document.documentElement.outerHTML").await?;
        let title = page.evaluate_as_string("document.title").await?;
        let final_url = page.evaluate_as_string("window.location.href").await?;

        // Extract cookies
        let cookies_raw = page.evaluate_as_string("document.cookie").await?;
        let cookies: Vec<String> = cookies_raw.split(';')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();

        // Determine status (approximate - check for common block indicators)
        let status = if html.contains("Access denied") || html.contains("403 Forbidden") {
            403
        } else if html.contains("Not Found") || html.contains("404") {
            404
        } else {
            200
        };

        browser.close().await?;

        Ok(ScrapeResponse {
            status,
            url: final_url,
            html,
            title,
            cookies,
        })
    }

    /// Launch a browser with optional proxy.
    async fn launch_browser(&self, proxy: Option<&str>) -> Result<Browser> {
        let proxy_config = proxy.map(|p| ProxyConfig {
            server: p.to_string(),
            username: None,
            password: None,
        });

        Browser::launch(LaunchOptions {
            headless: self.headless,
            proxy: proxy_config,
            ..Default::default()
        }).await
    }
}
