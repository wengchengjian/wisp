//! 统一 Cookie 存储 trait — 跨 HTTP/浏览器/CF 三处 cookie 状态。
//!
//! ARCH: 从 fetcher 下沉到 core，作为跨 crate 的领域契约。
//! 低层实现（BrowserCookieJar / HttpCookieJar / CfCookieJar）分别归属各自 crate，
//! 但共享此 trait 与 `Cookie` 类型。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use url::Url;

/// Cookie 表示（统一格式，跨 HTTP/浏览器/CF）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cookie {
    /// Cookie 名称。
    pub name: String,
    /// Cookie 值。
    pub value: String,
    /// Cookie 作用域名（如 "example.com"）。
    pub domain: String,
    /// Cookie 作用路径（如 "/"）。
    pub path: String,
    /// 是否仅 HTTPS 传输。
    pub secure: bool,
    /// 是否仅 HTTP 可访问（JS 不可读）。
    pub http_only: bool,
    /// SameSite 策略（"Strict"/"Lax"/"None"）。
    pub same_site: Option<String>,
    /// Unix 时间戳（秒），None 表示会话 cookie。
    pub expires: Option<f64>,
}

impl Cookie {
    /// 从 CDP `Network.getCookies` 返回的 JSON 对象转换。
    pub fn from_cdp_value(v: &serde_json::Value, default_domain: &str) -> Option<Self> {
        Some(Self {
            name: v.get("name")?.as_str()?.to_string(),
            value: v.get("value")?.as_str()?.to_string(),
            domain: v
                .get("domain")
                .and_then(|d| d.as_str())
                .unwrap_or(default_domain)
                .to_string(),
            path: v
                .get("path")
                .and_then(|p| p.as_str())
                .unwrap_or("/")
                .to_string(),
            secure: v
                .get("secure")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            http_only: v
                .get("httpOnly")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            same_site: v
                .get("sameSite")
                .and_then(|s| s.as_str())
                .map(std::string::ToString::to_string),
            expires: v.get("expires").and_then(serde_json::Value::as_f64),
        })
    }
}

/// Cookie 存储 trait — 统一 HTTP/浏览器/CF 三处 cookie 状态。
///
/// 实现归属：
/// - `BrowserCookieJar`（CDP）→ browser crate
/// - `HttpCookieJar`（wreq）→ http/fetcher crate
/// - `CfCookieJar`（moka + 文件）→ fetcher crate
#[async_trait]
pub trait CookieJar: Send + Sync {
    /// 获取指定 URL 的所有匹配 cookie（按 domain/path/secure 匹配）。
    async fn get(&self, url: &Url) -> Vec<Cookie>;

    /// 写入 cookie。
    async fn set(&self, cookie: Cookie);

    /// 批量写入 cookie。
    ///
    /// 默认实现逐个调用 `set`；后端若支持批量优化（如 CF 的单次持久化）应覆盖。
    async fn set_batch(&self, cookies: Vec<Cookie>) {
        for cookie in cookies {
            self.set(cookie).await;
        }
    }

    /// 删除指定 URL 匹配的所有 cookie（用于失效会话）。
    async fn clear(&self, url: &Url);

    /// 获取 Cookie 头字符串（用于 HTTP 请求注入）。
    /// 默认实现基于 `get()`，可被覆盖以优化。
    async fn header(&self, url: &Url) -> Option<String> {
        let cookies = self.get(url).await;
        if cookies.is_empty() {
            return None;
        }
        Some(
            cookies
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    /// 获取与 cookie 会话绑定的 User-Agent（如 CF 会话）。
    ///
    /// 默认返回 `None`；`CfCookieJar` 返回签发 cookie 时浏览器使用的实际 UA，
    /// 供 HTTP 快速路径保持 cookie 与 UA 一致性。
    async fn ua(&self, url: &Url) -> Option<String> {
        let _ = url;
        None
    }

    /// 设置与某域名 cookie 会话绑定的 User-Agent（默认 no-op）。
    async fn set_session_ua(&self, domain: &str, ua: Option<&str>) {
        let _ = (domain, ua);
    }

    /// 获取与 cookie 会话绑定的 sec-ch-ua 头（如 CF 会话）。
    ///
    /// 返回浏览器实际发送的完整 sec-ch-ua 值（如 `"Not/A)Brand";v="99", "Chromium";v="148"`）。
    /// 默认返回 `None`；`CfCookieJar` 返回签发 cookie 时浏览器捕获的真实值，
    /// 供 HTTP 快速路径保持 cookie 与 Client Hints 一致性。
    async fn sec_ch_ua(&self, url: &Url) -> Option<String> {
        let _ = url;
        None
    }

    /// 设置与某域名 cookie 会话绑定的 sec-ch-ua 头（默认 no-op）。
    async fn set_session_sec_ch_ua(&self, domain: &str, sec_ch_ua: Option<&str>) {
        let _ = (domain, sec_ch_ua);
    }

    /// 刷新某 URL 对应会话的活跃时间（默认 no-op）。
    ///
    /// 用途：HTTP 快速路径携带 CF cookie 复用成功时续期会话 TTL，避免 jar 的
    /// TTL（如 30 分钟）先于 cookie 实际有效期过期，导致 UA/sec-ch-ua 对齐信息
    /// 丢失、无谓回退 Stealth 重新挑战。`CfCookieJar` 覆盖实现为更新 `saved_at`
    /// 并重新插入以重启 moka TTL。
    async fn touch(&self, _url: &Url) {}
}

/// 测试用 MockCookieJar — 内存实现，记录所有操作。
pub struct MockCookieJar {
    cookies: parking_lot::Mutex<Vec<Cookie>>,
}

/// 域名匹配：host 必须等于 domain 或以 `.domain` 结尾，避免后缀撞名。
fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

impl MockCookieJar {
    /// 创建空 mock。
    #[must_use]
    pub fn new() -> Self {
        Self {
            cookies: parking_lot::Mutex::new(Vec::new()),
        }
    }
}

impl Default for MockCookieJar {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CookieJar for MockCookieJar {
    async fn get(&self, url: &Url) -> Vec<Cookie> {
        let host = url.host_str().unwrap_or("");
        let path = url.path();
        self.cookies
            .lock()
            .iter()
            .filter(|c| {
                // 简化匹配：域名边界匹配 + path 前缀匹配
                domain_matches(host, &c.domain) && path.starts_with(&c.path)
            })
            .cloned()
            .collect()
    }

    async fn set(&self, cookie: Cookie) {
        let mut guard = self.cookies.lock();
        // 替换同名同 domain 同 path 的 cookie
        guard.retain(|c| {
            !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
        });
        guard.push(cookie);
    }

    async fn clear(&self, url: &Url) {
        let host = url.host_str().unwrap_or("");
        let mut guard = self.cookies.lock();
        guard.retain(|c| !domain_matches(host, &c.domain));
    }
}
