//! Client builder.

use crate::{Client, Config};
use std::time::Duration;
use wreq::header::HeaderName;
use wreq_util::Profile;

use wisp_core::error::{NetworkError, Result, WispError};

/// Builder for Client.
pub struct ClientBuilder {
    pub(crate) config: Config,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientBuilder {
    /// 创建新的构建器。
    pub fn new() -> Self {
        Self {
            config: Config::default(),
        }
    }
    /// 设置请求超时。
    pub fn timeout(mut self, d: Duration) -> Self {
        self.config.timeout = d;
        self
    }
    /// 设置 User-Agent。
    pub fn user_agent(mut self, ua: &str) -> Self {
        self.config.user_agent = Some(ua.to_string());
        self
    }
    /// 设置代理。
    pub fn proxy(mut self, url: &str) -> Self {
        self.config.proxy = Some(url.to_string());
        self
    }
    /// 添加默认请求头。
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.config
            .headers
            .insert(key.to_string(), value.to_string());
        self
    }
    /// 设置最大重定向次数。
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

    /// ARCH: HttpCookieJar 自创建 `wreq::cookie::Jar`，通过此方法注入到 wreq::Client，
    /// 实现 HttpCookieJar 与 wreq::Client 自动 cookie 管理共享同一个 jar。
    #[must_use]
    pub fn cookie_provider(mut self, jar: std::sync::Arc<wreq::cookie::Jar>) -> Self {
        self.config.cookie_jar = Some(jar);
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

    /// ND-011-SEC：禁用 TLS 证书验证（危险！）。
    ///
    /// 仅用于测试或抓取自签名证书的内部站点。启用后存在中间人攻击风险，
    /// 生产环境应保持默认 false（启用验证）。
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.config.danger_accept_invalid_certs = accept;
        self
    }

    /// 获取配置引用（测试用）
    #[doc(hidden)]
    pub fn config_ref(&self) -> &Config {
        &self.config
    }

    /// 构建 HTTP 客户端。
    pub fn build(mut self) -> Result<Client> {
        let mut builder = wreq::Client::builder()
            .timeout(self.config.timeout)
            .redirect(wreq::redirect::Policy::limited(self.config.max_redirects))
            // 重试由 wisp Engine 统一管理（fetch_dispatch + RetryMiddleware），
            // 关闭 wreq 默认的每请求 2 次协议重试层，避免重复重试和 tower 层开销。
            .retry(wreq::retry::Policy::never())
            .tls_cert_verification(!self.config.danger_accept_invalid_certs);

        // 优先使用外部注入的 cookie jar（HttpCookieJar），否则启用 wreq 内置 cookie_store
        if let Some(jar) = self.config.cookie_jar.take() {
            builder = builder.cookie_provider(jar);
        } else {
            builder = builder.cookie_store(true);
        }

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
        // DoH：代理场景下防止 DNS 泄漏，使用自定义 resolver 替换系统解析。
        if let Some(ref doh) = self.config.dns_over_https {
            let resolver = crate::dns::DoHResolver::new(doh)?;
            builder = builder.dns_resolver(resolver);
        }
        // 注：wreq 6.0.0-rc.29 ClientBuilder 未暴露 headers_order 方法，
        // header_order 字段暂不应用，保留供未来版本支持后启用

        let http_client = builder.build().map_err(|e| {
            WispError::Network(NetworkError::Http(format!("client build error: {e}")))
        })?;

        Ok(Client {
            http: http_client,
            config: self.config,
        })
    }
}
