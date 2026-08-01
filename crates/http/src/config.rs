//! HTTP client configuration.

use std::collections::HashMap;
use std::time::Duration;
use wreq::header::HeaderName;
use wreq_util::Profile;

/// HTTP client configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// 请求超时。
    pub timeout: Duration,
    /// 自定义 User-Agent。
    pub user_agent: Option<String>,
    /// 默认请求头。
    pub headers: HashMap<String, String>,
    /// 代理地址。
    pub proxy: Option<String>,
    /// 最大重定向次数。
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
    /// ND-011-SEC：是否禁用 TLS 证书验证（危险！仅用于测试或自签名证书内部站点）。
    /// 默认 false（启用验证）。设为 true 等价于 curl -k，存在中间人攻击风险。
    pub danger_accept_invalid_certs: bool,
    /// 外部 cookie jar（HttpCookieJar 注入）。
    /// `None` 时使用 wreq::Client 内置 `cookie_store`。
    pub cookie_jar: Option<std::sync::Arc<wreq::cookie::Jar>>,
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
            danger_accept_invalid_certs: false,
            cookie_jar: None,
        }
    }
}
