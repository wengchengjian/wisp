//! FetchClient 配置。

use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::time::Duration;

use wisp_http::block::DomainBlocker;
use wisp_http::Config as HttpConfig;

/// 统一请求客户端配置。
#[derive(Debug, Clone)]
pub struct FetchClientConfig {
    /// HTTP 传输配置（超时/UA/headers/代理/重定向/指纹/DoH/body 限制/证书校验）。
    pub http: HttpConfig,
    /// 浏览器 headless 模式
    pub headless: bool,
    /// 浏览器可执行文件路径（None = 自动搜索 Chrome/Chromium/Edge）
    pub executable_path: Option<std::path::PathBuf>,
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
    /// BrowserPool 最大并发 page 数（0 = 禁用浏览器模式）
    pub max_concurrent_pages: usize,
    /// Turnstile 解决器参数配置。
    #[cfg(feature = "stealth")]
    pub turnstile: wisp_stealth::TurnstileConfig,
    /// CF 会话缓存 TTL（默认 30 分钟）。
    pub cf_cookie_ttl: Duration,
    /// CF 会话持久化目录（默认 "wisp-data"）。
    pub cf_data_dir: PathBuf,
}

impl Deref for FetchClientConfig {
    type Target = HttpConfig;
    fn deref(&self) -> &HttpConfig {
        &self.http
    }
}

impl DerefMut for FetchClientConfig {
    fn deref_mut(&mut self) -> &mut HttpConfig {
        &mut self.http
    }
}
impl Default for FetchClientConfig {
    fn default() -> Self {
        let mut http = HttpConfig::default();
        // 保持旧行为：不显式设置 UA 时由底层决定，而非注入 http::Config 默认 UA。
        http.user_agent = None;
        Self {
            http,
            headless: true,
            executable_path: None,
            human_mode: true,
            challenge_timeout: Duration::from_secs(30),
            wait_for: None,
            extra_wait_ms: 0,
            domain_blocker: None,
            max_concurrent_pages: 4,
            #[cfg(feature = "stealth")]
            turnstile: wisp_stealth::TurnstileConfig::default(),
            cf_cookie_ttl: Duration::from_mins(30),
            cf_data_dir: PathBuf::from("wisp-data"),
        }
    }
}
