//! HTTP client with automatic encoding detection.
//!
//! Wraps reqwest with builder pattern, proxy support, and HTML parsing.

pub mod encoding;

use std::collections::HashMap;
use std::time::Duration;
use serde_json::Value;

use crate::error::{WispError, Result};
use crate::parser::Node;

/// HTTP client configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub timeout: Duration,
    pub user_agent: Option<String>,
    pub headers: HashMap<String, String>,
    pub proxy: Option<String>,
    pub max_redirects: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            user_agent: Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36".to_string()),
            headers: HashMap::new(),
            proxy: None,
            max_redirects: 10,
        }
    }
}

/// Builder for Client.
pub struct ClientBuilder {
    config: Config,
}

impl ClientBuilder {
    pub fn new() -> Self { Self { config: Config::default() } }
    pub fn timeout(mut self, d: Duration) -> Self { self.config.timeout = d; self }
    pub fn user_agent(mut self, ua: &str) -> Self { self.config.user_agent = Some(ua.to_string()); self }
    pub fn proxy(mut self, url: &str) -> Self { self.config.proxy = Some(url.to_string()); self }
    pub fn header(mut self, key: &str, value: &str) -> Self { self.config.headers.insert(key.to_string(), value.to_string()); self }
    pub fn max_redirects(mut self, n: usize) -> Self { self.config.max_redirects = n; self }

    pub fn build(self) -> Result<Client> {
        let mut builder = reqwest::Client::builder()
            .timeout(self.config.timeout)
            .redirect(reqwest::redirect::Policy::limited(self.config.max_redirects))
            .danger_accept_invalid_certs(false);

        if let Some(ref ua) = self.config.user_agent {
            builder = builder.user_agent(ua);
        }
        if let Some(ref proxy_url) = self.config.proxy {
            let proxy = reqwest::Proxy::all(proxy_url)
                .map_err(|e| WispError::CdpError(format!("proxy error: {e}")))?;
            builder = builder.proxy(proxy);
        }

        let http_client = builder.build()
            .map_err(|e| WispError::CdpError(format!("client build error: {e}")))?;

        Ok(Client { http: http_client, config: self.config })
    }
}

/// HTTP client for fetching web pages.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    config: Config,
}

impl Client {
    pub fn builder() -> ClientBuilder { ClientBuilder::new() }

    /// Create a client with default config.
    pub fn new() -> Result<Self> { ClientBuilder::new().build() }

    /// GET request.
    pub async fn get(&self, url: &str) -> Result<Response> {
        let resp = self.http.get(url)
            .headers(self.build_headers())
            .send().await
            .map_err(|e| WispError::CdpError(format!("GET {url}: {e}")))?;
        self.build_response(resp).await
    }

    /// POST request with optional body/json.
    pub async fn post(&self, url: &str, body: Option<&str>, json: Option<&Value>) -> Result<Response> {
        let mut req = self.http.post(url).headers(self.build_headers());
        if let Some(b) = body { req = req.body(b.to_string()); }
        if let Some(j) = json {
            let json_str = serde_json::to_string(j)
                .map_err(|e| WispError::CdpError(format!("JSON serialize: {e}")))?;
            req = req.header(reqwest::header::CONTENT_TYPE, "application/json").body(json_str);
        }
        let resp = req.send().await
            .map_err(|e| WispError::CdpError(format!("POST {url}: {e}")))?;
        self.build_response(resp).await
    }

    /// PUT request.
    pub async fn put(&self, url: &str, body: Option<&str>, json: Option<&Value>) -> Result<Response> {
        let mut req = self.http.put(url).headers(self.build_headers());
        if let Some(b) = body { req = req.body(b.to_string()); }
        if let Some(j) = json {
            let json_str = serde_json::to_string(j)
                .map_err(|e| WispError::CdpError(format!("JSON serialize: {e}")))?;
            req = req.header(reqwest::header::CONTENT_TYPE, "application/json").body(json_str);
        }
        let resp = req.send().await
            .map_err(|e| WispError::CdpError(format!("PUT {url}: {e}")))?;
        self.build_response(resp).await
    }

    /// DELETE request.
    pub async fn delete(&self, url: &str) -> Result<Response> {
        let resp = self.http.delete(url)
            .headers(self.build_headers())
            .send().await
            .map_err(|e| WispError::CdpError(format!("DELETE {url}: {e}")))?;
        self.build_response(resp).await
    }

    fn build_headers(&self) -> reqwest::header::HeaderMap {
        let mut map = reqwest::header::HeaderMap::new();
        for (k, v) in &self.config.headers {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                map.insert(name, val);
            }
        }
        map
    }

    async fn build_response(&self, resp: reqwest::Response) -> Result<Response> {
        let status = resp.status().as_u16();
        let url = resp.url().to_string();
        let content_type = resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let headers: HashMap<String, String> = resp.headers().iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.to_string(), s.to_string())))
            .collect();
        let body = resp.bytes().await
            .map_err(|e| WispError::CdpError(format!("read body: {e}")))?
            .to_vec();

        Ok(Response { status, url, headers, body, content_type })
    }
}

/// HTTP response with parsing helpers.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    content_type: String,
}

impl Response {
    /// Decode body as text with automatic charset detection.
    pub fn text(&self) -> Result<String> {
        Ok(encoding::decode(&self.body, &self.content_type))
    }

    /// Parse body as JSON.
    pub fn json(&self) -> Result<Value> {
        let text = self.text()?;
        serde_json::from_str(&text)
            .map_err(|e| WispError::CdpError(format!("JSON parse: {e}")))
    }

    /// Parse body as HTML into a Node.
    pub fn parse(&self) -> Result<Node> {
        let text = self.text()?;
        Ok(Node::from_html(&text))
    }

    pub fn is_ok(&self) -> bool { self.status >= 200 && self.status < 300 }
}
