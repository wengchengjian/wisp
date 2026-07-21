//! HTTP client with automatic encoding detection.
//!
//! Wraps wreq with builder pattern, proxy support, and HTML parsing.

pub mod encoding;
pub mod proxy;

use std::collections::HashMap;
use std::time::Duration;
use serde_json::Value;
use wreq::header::HeaderName;
use wreq_util::Profile;

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
    /// 浏览器 TLS 指纹模拟（默认 Chrome136，覆盖最广）
    pub emulation: Option<Profile>,
    /// 自定义 header 顺序（wreq 6.0.0-rc.29 已移除 headers_order 方法，字段保留供未来扩展）
    pub header_order: Option<Vec<HeaderName>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            user_agent: Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36".to_string()),
            headers: HashMap::new(),
            proxy: None,
            max_redirects: 10,
            // 默认 Chrome 136 指纹（覆盖最广）
            emulation: Some(Profile::Chrome136),
            header_order: None,
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

    /// 指定浏览器 TLS 指纹模拟（Chrome/Firefox/Safari/Edge/OkHttp，75 变体）
    pub fn emulation(mut self, emu: Profile) -> Self {
        self.config.emulation = Some(emu);
        self
    }

    /// 关闭 TLS 指纹模拟（用 wreq 默认行为，用于调试）
    pub fn no_emulation(mut self) -> Self {
        self.config.emulation = None;
        self
    }

    /// 自定义 header 顺序（wreq 6.0.0-rc.29 已移除 headers_order 方法，配置保留供未来扩展）
    pub fn header_order(mut self, order: Vec<HeaderName>) -> Self {
        self.config.header_order = Some(order);
        self
    }

    /// 获取配置引用（测试用）
    #[doc(hidden)]
    pub fn config_ref(&self) -> &Config {
        &self.config
    }

    pub fn build(self) -> Result<Client> {
        let mut builder = wreq::Client::builder()
            .timeout(self.config.timeout)
            .redirect(wreq::redirect::Policy::limited(self.config.max_redirects))
            .tls_cert_verification(true);

        if let Some(ref ua) = self.config.user_agent {
            builder = builder.user_agent(ua);
        }
        if let Some(ref proxy_url) = self.config.proxy {
            let proxy = wreq::Proxy::all(proxy_url)
                .map_err(|e| WispError::CdpError(format!("proxy error: {e}")))?;
            builder = builder.proxy(proxy);
        }
        // 应用 TLS 指纹模拟（wreq 文档说明会覆盖现有 TLS/HTTP2 配置）
        if let Some(emu) = self.config.emulation {
            builder = builder.emulation(emu);
        }
        // 注：wreq 6.0.0-rc.29 已移除 headers_order 方法，header_order 字段暂不应用

        let http_client = builder.build()
            .map_err(|e| WispError::CdpError(format!("client build error: {e}")))?;

        Ok(Client { http: http_client, config: self.config })
    }
}

/// HTTP client for fetching web pages.
#[derive(Clone)]
pub struct Client {
    http: wreq::Client,
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
            req = req.header(wreq::header::CONTENT_TYPE, "application/json").body(json_str);
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
            req = req.header(wreq::header::CONTENT_TYPE, "application/json").body(json_str);
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

    fn build_headers(&self) -> wreq::header::HeaderMap {
        let mut map = wreq::header::HeaderMap::new();
        for (k, v) in &self.config.headers {
            if let (Ok(name), Ok(val)) = (
                wreq::header::HeaderName::from_bytes(k.as_bytes()),
                wreq::header::HeaderValue::from_str(v),
            ) {
                map.insert(name, val);
            }
        }
        map
    }

    async fn build_response(&self, resp: wreq::Response) -> Result<Response> {
        let status = resp.status().as_u16();
        let url = resp.uri().to_string();
        let content_type = resp.headers()
            .get(wreq::header::CONTENT_TYPE)
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
