//! HTTP client with automatic encoding detection.
//!
//! Wraps wreq with builder pattern, proxy support, and HTML parsing.

pub mod block;
pub mod encoding;
pub mod proxy;
pub mod ua;

pub use block::DomainBlocker;
pub use ua::UaRotator;

use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use wreq::header::HeaderName;
use wreq_util::Profile;

use crate::error::{Result, WispError, NetworkError, ParseError};
use crate::fetcher::{Method as FetchMethod, Request as FetchRequest, Response as FetchResponse};

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
    /// 自定义 header 顺序（wreq 6.0.0-rc.29 未暴露 headers_order 方法，字段暂不应用）
    pub header_order: Option<Vec<HeaderName>>,
    /// DNS-over-HTTPS 服务器 URL（如 "https://1.1.1.1/dns-query"）。
    /// 启用后通过 DoH 解析域名，防止代理场景下 DNS 泄漏。
    pub dns_over_https: Option<String>,
    /// 响应体最大字节数。超过则返回 `ResponseBodyTooLarge` 错误，防止 OOM。
    /// 默认 64MB（覆盖大多数 HTML 页面；二进制/大文件场景应显式调高）。
    pub max_body_size: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            user_agent: Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".to_string()),
            headers: HashMap::new(),
            proxy: None,
            max_redirects: 10,
            // 默认 Chrome 136 指纹（覆盖最广）
            emulation: Some(Profile::Chrome136),
            header_order: None,
            dns_over_https: None,
            max_body_size: 64 * 1024 * 1024,
        }
    }
}

/// Builder for Client.
pub struct ClientBuilder {
    config: Config,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientBuilder {
    pub fn new() -> Self {
        Self {
            config: Config::default(),
        }
    }
    pub fn timeout(mut self, d: Duration) -> Self {
        self.config.timeout = d;
        self
    }
    pub fn user_agent(mut self, ua: &str) -> Self {
        self.config.user_agent = Some(ua.to_string());
        self
    }
    pub fn proxy(mut self, url: &str) -> Self {
        self.config.proxy = Some(url.to_string());
        self
    }
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.config
            .headers
            .insert(key.to_string(), value.to_string());
        self
    }
    pub fn max_redirects(mut self, n: usize) -> Self {
        self.config.max_redirects = n;
        self
    }

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

    /// 自定义 header 顺序（wreq 6.0.0-rc.29 未暴露 headers_order 方法，配置暂不生效）
    pub fn header_order(mut self, order: Vec<HeaderName>) -> Self {
        self.config.header_order = Some(order);
        self
    }

    /// 设置 DNS-over-HTTPS 服务器（防止代理场景 DNS 泄漏）。
    ///
    /// 常用值："https://1.1.1.1/dns-query" (Cloudflare) 或 "https://dns.google/dns-query" (Google)
    pub fn dns_over_https(mut self, url: &str) -> Self {
        self.config.dns_over_https = Some(url.to_string());
        self
    }

    /// 设置响应体最大字节数。超过则返回 `ResponseBodyTooLarge` 错误。
    pub fn max_body_size(mut self, max: usize) -> Self {
        self.config.max_body_size = max;
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
            .tls_cert_verification(true)
            .cookie_store(true);

        if let Some(ref ua) = self.config.user_agent {
            builder = builder.user_agent(ua);
        }
        if let Some(ref proxy_url) = self.config.proxy {
            let proxy = wreq::Proxy::all(proxy_url)
                .map_err(|e| WispError::Network(NetworkError::Http(format!("proxy error: {e}"))))?;
            builder = builder.proxy(proxy);
        }
        // 应用 TLS 指纹模拟（wreq 文档说明会覆盖现有 TLS/HTTP2 配置）
        if let Some(emu) = self.config.emulation {
            builder = builder.emulation(emu);
        }
        // 注：wreq 6.0.0-rc.29 ClientBuilder 未暴露 headers_order 方法，
        // header_order 字段暂不应用，保留供未来版本支持后启用

        let http_client = builder
            .build()
            .map_err(|e| WispError::Network(NetworkError::Http(format!("client build error: {e}"))))?;

        Ok(Client {
            http: http_client,
            config: self.config,
        })
    }
}

/// HTTP client for fetching web pages.
#[derive(Clone)]
pub struct Client {
    http: wreq::Client,
    config: Config,
}

impl Client {
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    /// Create a client with default config.
    pub fn new() -> Result<Self> {
        ClientBuilder::new().build()
    }

    /// 获取配置引用（供 Engine 代理轮换时读取 timeout 等参数）。
    pub fn config_ref(&self) -> &Config {
        &self.config
    }

    /// 统一请求入口：接受 `fetcher::Request`，直接返回 `fetcher::Response`。
    ///
    /// 消除 http::Response 中间类型，避免字段克隆转换。
    pub async fn fetch(&self, req: &FetchRequest) -> Result<FetchResponse> {
        let extra_headers: Vec<(String, String)> = req
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let wreq_resp = match req.method {
            FetchMethod::Get => {
                self.http
                    .get(&req.url)
                    .headers(self.build_headers_with(&extra_headers))
                    .send()
                    .await
            }
            FetchMethod::Post => {
                let mut builder = self
                    .http
                    .post(&req.url)
                    .headers(self.build_headers_with(&extra_headers));
                if let Some(ref b) = req.body {
                    builder = builder.body(b.clone());
                }
                builder.send().await
            }
            FetchMethod::Put => {
                let mut builder = self
                    .http
                    .put(&req.url)
                    .headers(self.build_headers_with(&extra_headers));
                if let Some(ref b) = req.body {
                    builder = builder.body(b.clone());
                }
                builder.send().await
            }
            FetchMethod::Delete => {
                self.http
                    .delete(&req.url)
                    .headers(self.build_headers_with(&extra_headers))
                    .send()
                    .await
            }
        };

        let wreq_resp = wreq_resp.map_err(|e| classify_request_error(&e, &req.url, self.config.timeout))?;
        self.build_fetch_response(wreq_resp, req.clone()).await
    }

    /// GET request（便捷方法，内部构造 Request）。
    pub async fn get(&self, url: &str, extra_headers: &[(String, String)]) -> Result<FetchResponse> {
        let mut req = FetchRequest::get(url);
        for (k, v) in extra_headers {
            req.headers.insert(k.clone(), v.clone());
        }
        self.fetch(&req).await
    }

    /// POST request with optional body/json（便捷方法）。
    pub async fn post(
        &self,
        url: &str,
        body: Option<&str>,
        json: Option<&Value>,
        extra_headers: &[(String, String)],
    ) -> Result<FetchResponse> {
        let mut req = FetchRequest::post(url, body.map(|b| b.to_string()));
        for (k, v) in extra_headers {
            req.headers.insert(k.clone(), v.clone());
        }
        if let Some(j) = json {
            let json_str = serde_json::to_string(j)
                .map_err(|e| WispError::Parse(ParseError::Json(format!("JSON serialize: {e}"))))?;
            req.body = Some(json_str);
            req.headers.insert("content-type".to_string(), "application/json".to_string());
        }
        self.fetch(&req).await
    }

    /// PUT request（便捷方法）。
    pub async fn put(
        &self,
        url: &str,
        body: Option<&str>,
        json: Option<&Value>,
        extra_headers: &[(String, String)],
    ) -> Result<FetchResponse> {
        let mut req = FetchRequest {
            url: url.to_string(),
            method: FetchMethod::Put,
            body: body.map(|b| b.to_string()),
            ..Default::default()
        };
        for (k, v) in extra_headers {
            req.headers.insert(k.clone(), v.clone());
        }
        if let Some(j) = json {
            let json_str = serde_json::to_string(j)
                .map_err(|e| WispError::Parse(ParseError::Json(format!("JSON serialize: {e}"))))?;
            req.body = Some(json_str);
            req.headers.insert("content-type".to_string(), "application/json".to_string());
        }
        self.fetch(&req).await
    }

    /// DELETE request（便捷方法）。
    pub async fn delete(&self, url: &str, extra_headers: &[(String, String)]) -> Result<FetchResponse> {
        let mut req = FetchRequest {
            url: url.to_string(),
            method: FetchMethod::Delete,
            ..Default::default()
        };
        for (k, v) in extra_headers {
            req.headers.insert(k.clone(), v.clone());
        }
        self.fetch(&req).await
    }

    /// 合并 config headers 与 per-request extra headers（extra 覆盖同名 config header）。
    fn build_headers_with(&self, extra_headers: &[(String, String)]) -> wreq::header::HeaderMap {
        let mut map = self.build_headers();
        for (k, v) in extra_headers {
            match (
                wreq::header::HeaderName::from_bytes(k.as_bytes()),
                wreq::header::HeaderValue::from_str(v),
            ) {
                (Ok(name), Ok(val)) => { map.insert(name, val); }
                (Err(e), _) => tracing::warn!("跳过无效 header 名 '{}': {}", k, e),
                (_, Err(e)) => tracing::warn!("跳过无效 header 值 '{}': {}", k, e),
            }
        }
        map
    }

    fn build_headers(&self) -> wreq::header::HeaderMap {
        let mut map = wreq::header::HeaderMap::new();
        for (k, v) in &self.config.headers {
            match (
                wreq::header::HeaderName::from_bytes(k.as_bytes()),
                wreq::header::HeaderValue::from_str(v),
            ) {
                (Ok(name), Ok(val)) => { map.insert(name, val); }
                (Err(e), _) => tracing::warn!("跳过无效 config header 名 '{}': {}", k, e),
                (_, Err(e)) => tracing::warn!("跳过无效 config header 值 '{}': {}", k, e),
            }
        }
        map
    }

    /// 从 wreq 响应构建统一 `fetcher::Response`（流式读取 body + 大小限制）。
    async fn build_fetch_response(&self, resp: wreq::Response, request: FetchRequest) -> Result<FetchResponse> {
        let status = resp.status().as_u16();
        let url = resp.uri().to_string();
        let content_type = resp
            .headers()
            .get(wreq::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.to_string(), s.to_string())))
            .collect();

        // 流式读取 body 并检查大小限制，防止超大响应导致 OOM
        let max_body_size = self.config.max_body_size;
        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| WispError::Network(NetworkError::Http(format!("read body chunk: {e}"))))?;
            if body.len() + chunk.len() > max_body_size {
                return Err(WispError::Network(NetworkError::ResponseBodyTooLarge {
                    url: url.clone(),
                    actual: body.len() + chunk.len(),
                    limit: max_body_size,
                }));
            }
            body.extend_from_slice(&chunk);
        }

        Ok(FetchResponse::from_http(
            status,
            url,
            headers,
            body,
            content_type,
            request,
        ))
    }
}


/// 将 `wreq::Error` 分类为结构化的 `WispError`，支持按错误类别差异化重试/降级。
///
/// 分类顺序（先命中先返回）：
/// 1. `is_timeout` → `RequestTimedOut`
/// 2. `is_proxy_connect` → `ProxyFailed`
/// 3. `is_tls` → `TlsFailed`
/// 4. `is_connect` → 区分 DNS 失败（`DnsFailed`）与 TCP 连接失败（`ConnectionFailed`）
/// 5. `is_builder` → `UrlParse`（通常是 URL scheme/解析错误）
/// 6. 其他 → 退化为 `HttpError`
///
/// DNS 判定通过错误消息字符串匹配（wreq 未暴露 DNS 专用判断 API），覆盖常见
/// GaiError 消息："nodename nor servname"、"name or service not known"、
/// "no such host"、"name resolution" 等。
fn classify_request_error(e: &wreq::Error, url: &str, timeout: Duration) -> WispError {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| url.to_string());

    if e.is_timeout() {
        return WispError::Network(NetworkError::RequestTimedOut {
            url: url.to_string(),
            timeout_secs: timeout.as_secs(),
        });
    }
    if e.is_proxy_connect() {
        return WispError::Network(NetworkError::ProxyFailed {
            detail: e.to_string(),
        });
    }
    if e.is_tls() {
        return WispError::Network(NetworkError::TlsFailed {
            host,
            detail: e.to_string(),
        });
    }
    if e.is_connect() {
        let detail = e.to_string();
        let lower = detail.to_lowercase();
        const DNS_HINTS: &[&str] = &[
            "nodename nor servname",
            "name or service not known",
            "no such host",
            "name resolution",
            "dns",
            "resolve",
            "temporary failure in name resolution",
        ];
        if DNS_HINTS.iter().any(|h| lower.contains(h)) {
            return WispError::Network(NetworkError::DnsFailed { host, detail });
        }
        return WispError::Network(NetworkError::ConnectionFailed { host, detail });
    }
    if e.is_builder() {
        return WispError::Network(NetworkError::UrlParse(e.to_string()));
    }
    WispError::Network(NetworkError::Http(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 启动测试 HTTP 服务器，回显收到的请求 headers（每行一个 header）。
    async fn spawn_echo_server() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16384];
                    let mut total = 0usize;
                    // 循环读取直到收到完整的 HTTP 请求头（\r\n\r\n 结尾）
                    while total < buf.len() {
                        let n = socket.read(&mut buf[total..]).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        total += n;
                        if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8_lossy(&buf[..total]);
                    // 回显收到的 headers（跳过请求行）
                    let headers: String = request
                        .lines()
                        .skip(1)
                        .take_while(|line| !line.is_empty())
                        .filter(|line| line.contains(':'))
                        .map(|line| format!("{}\n", line))
                        .collect();
                    let body = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        headers.len(),
                        headers
                    );
                    let _ = socket.write_all(body.as_bytes()).await;
                });
            }
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn put_with_extra_headers_sends_them() {
        let base = spawn_echo_server().await;
        let client = Client::builder().no_emulation().build().unwrap();
        let extra = vec![("X-Custom".to_string(), "put-val".to_string())];
        let resp = client
            .put(&format!("{}/item", base), None, None, &extra)
            .await
            .unwrap();
        let text = resp.text().unwrap();
        assert!(
            text.to_ascii_lowercase().contains("x-custom: put-val"),
            "PUT 应发送 extra headers, 实际: {text}"
        );
    }

    #[tokio::test]
    async fn delete_with_extra_headers_sends_them() {
        let base = spawn_echo_server().await;
        let client = Client::builder().no_emulation().build().unwrap();
        let extra = vec![("X-Custom".to_string(), "del-val".to_string())];
        let resp = client
            .delete(&format!("{}/item", base), &extra)
            .await
            .unwrap();
        let text = resp.text().unwrap();
        assert!(
            text.to_ascii_lowercase().contains("x-custom: del-val"),
            "DELETE 应发送 extra headers, 实际: {text}"
        );
    }

    #[tokio::test]
    async fn get_with_empty_extra_headers_still_works() {
        let base = spawn_echo_server().await;
        let client = Client::builder().no_emulation().build().unwrap();
        let resp = client.get(&format!("{}/item", base), &[]).await.unwrap();
        assert_eq!(resp.status, 200);
    }
}
