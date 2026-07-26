//! CF 会话 cookie jar — moka::Cache + 文件持久化。
//!
//! ARCH: 从 FetchClient 迁出，由 StealthStrategy 持有。
//! 保留两级缓存（moka 内存 + JSON 文件），TTL 默认 30 分钟。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use moka::sync::Cache;
use url::Url;

use crate::cookie::{Cookie, CookieJar};

/// CF 会话条目：cookie + UA 绑定存储。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CfSession {
    /// CDP 返回的原始 cookie JSON 数组（含 name/value/domain/path/secure/httpOnly/sameSite 等）。
    pub cookies: Vec<serde_json::Value>,
    /// 浏览器实际 UA（CF 挑战解决时捕获，复用给后续 HTTP 请求）。
    pub ua: String,
    /// Unix 时间戳（秒），用于文件加载时判断过期。
    pub saved_at: i64,
}

/// CF 会话 cookie jar：moka 内存热缓存 + 本地 JSON 文件持久化。
///
/// - 读取：moka 优先（TTL 由 moka 管理）
/// - 写入：moka + 文件双写（write-through）
/// - 启动：从文件加载未过期条目到 moka
pub struct CfCookieJar {
    mem: Cache<String, CfSession>,
    file_path: PathBuf,
}

impl CfCookieJar {
    /// 创建 jar：从文件加载未过期条目到 moka。
    #[must_use]
    pub fn new(data_dir: &Path, ttl: Duration) -> Self {
        let file_path = data_dir.join("cf_sessions.json");
        let mem: Cache<String, CfSession> =
            Cache::builder().time_to_live(ttl).max_capacity(64).build();

        let cache = Self { mem, file_path };
        cache.load_from_file(ttl);
        cache
    }

    /// 读取 CF 会话（moka 优先，启动时已批量加载文件）。
    #[must_use]
    pub fn get_session(&self, domain: &str) -> Option<CfSession> {
        self.mem.get(domain)
    }

    /// 写入 CF 会话（moka + 文件双写）。
    pub fn insert_session(&self, domain: String, session: CfSession) {
        self.mem.insert(domain, session);
        self.save_to_file();
    }

    /// 文件加载：启动时调用，跳过过期条目。
    fn load_from_file(&self, ttl: Duration) {
        let content = match std::fs::read_to_string(&self.file_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let map: HashMap<String, CfSession> = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("CF 会话文件解析失败，忽略: {e}");
                return;
            }
        };
        let now = chrono::Utc::now().timestamp();
        let ttl_secs = ttl.as_secs() as i64;
        let mut loaded = 0u32;
        for (domain, session) in map {
            if now - session.saved_at < ttl_secs {
                self.mem.insert(domain, session);
                loaded += 1;
            }
        }
        if loaded > 0 {
            tracing::info!("CF 会话缓存: 从文件恢复 {loaded} 个域名的会话");
        }
    }

    /// 文件持久化：全量写入当前 moka 中所有条目。
    fn save_to_file(&self) {
        if let Some(parent) = self.file_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut map = HashMap::new();
        for (domain, session) in &self.mem {
            map.insert(domain.to_string(), session.clone());
        }
        match serde_json::to_string_pretty(&map) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.file_path, json) {
                    tracing::warn!("CF 会话文件写入失败: {e}");
                }
            }
            Err(e) => tracing::warn!("CF 会话序列化失败: {e}"),
        }
    }

    /// 从 serde_json::Value 提取 Cookie（CDP Network.getCookies 返回格式）。
    fn value_to_cookie(v: &serde_json::Value, default_domain: &str) -> Option<Cookie> {
        Some(Cookie {
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
            expires: v
                .get("expires")
                .and_then(serde_json::Value::as_f64),
        })
    }
}

#[async_trait]
impl CookieJar for CfCookieJar {
    async fn get(&self, url: &Url) -> Vec<Cookie> {
        let Some(domain) = url.host_str() else {
            return Vec::new();
        };
        let Some(session) = self.get_session(domain) else {
            return Vec::new();
        };
        session
            .cookies
            .iter()
            .filter_map(|v| Self::value_to_cookie(v, domain))
            .collect()
    }

    async fn set(&self, cookie: Cookie) {
        // CfCookieJar 按 domain 索引整个会话，set 单个 cookie 时合并到现有会话
        let domain = cookie.domain.clone();
        let cookie_json = serde_json::json!({
            "name": cookie.name,
            "value": cookie.value,
            "domain": cookie.domain,
            "path": cookie.path,
            "secure": cookie.secure,
            "httpOnly": cookie.http_only,
            "sameSite": cookie.same_site.clone().unwrap_or_else(|| "Lax".into()),
            "expires": cookie.expires,
        });
        let mut session = self.get_session(&domain).unwrap_or_else(|| CfSession {
            cookies: Vec::new(),
            ua: String::new(),
            saved_at: chrono::Utc::now().timestamp(),
        });
        // 替换同名 cookie
        session
            .cookies
            .retain(|c| c.get("name").and_then(|n| n.as_str()) != Some(&cookie.name));
        session.cookies.push(cookie_json);
        session.saved_at = chrono::Utc::now().timestamp();
        self.insert_session(domain, session);
    }

    async fn clear(&self, url: &Url) {
        if let Some(domain) = url.host_str() {
            self.mem.invalidate(domain);
            self.save_to_file();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_url(s: &str) -> Url {
        Url::parse(s).expect("合法 URL")
    }

    /// 创建临时目录 + CfCookieJar。
    fn make_jar() -> (CfCookieJar, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let jar = CfCookieJar::new(dir.path(), Duration::from_mins(1));
        (jar, dir)
    }

    #[tokio::test]
    async fn cf_set_and_get_cookie() {
        let (jar, _dir) = make_jar();
        let cookie = Cookie {
            name: "cf_clearance".into(),
            value: "abc123".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: true,
            http_only: true,
            same_site: Some("Lax".into()),
            expires: None,
        };
        jar.set(cookie).await;

        let url = make_url("https://example.com/path");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "cf_clearance");
        assert_eq!(cookies[0].value, "abc123");
    }

    #[tokio::test]
    async fn cf_clear_removes_session() {
        let (jar, _dir) = make_jar();
        let cookie = Cookie {
            name: "cf_clearance".into(),
            value: "v".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;

        let url = make_url("https://example.com/");
        assert!(!jar.get(&url).await.is_empty());

        jar.clear(&url).await;
        assert!(jar.get(&url).await.is_empty());
    }

    #[tokio::test]
    async fn cf_header_returns_string() {
        let (jar, _dir) = make_jar();
        let cookie = Cookie {
            name: "cf_clearance".into(),
            value: "token123".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;

        let url = make_url("https://example.com/");
        let header = jar.header(&url).await;
        assert_eq!(header.as_deref(), Some("cf_clearance=token123"));
    }

    #[tokio::test]
    async fn cf_persist_and_reload() {
        let (jar, dir) = make_jar();
        let cookie = Cookie {
            name: "cf_clearance".into(),
            value: "persisted".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;

        // 文件应存在
        let file_path = dir.path().join("cf_sessions.json");
        assert!(file_path.exists(), "持久化文件应存在");

        // 重新加载，cookie 应恢复（TTL 内）
        let jar2 = CfCookieJar::new(dir.path(), Duration::from_mins(1));
        let url = make_url("https://example.com/");
        let cookies = jar2.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].value, "persisted");
    }

    #[tokio::test]
    async fn cf_expired_session_not_reloaded() {
        let (jar, dir) = make_jar();
        let cookie = Cookie {
            name: "cf_clearance".into(),
            value: "expired".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;

        // 用极短 TTL 重新加载，文件中的 saved_at 已过期
        let jar2 = CfCookieJar::new(dir.path(), Duration::from_millis(1));
        tokio::time::sleep(Duration::from_millis(10)).await;
        let url = make_url("https://example.com/");
        // 注意：moka 自身的 TTL 在加载时已生效，过期条目不会进入 moka
        let cookies = jar2.get(&url).await;
        // 加载时 saved_at 与当前时间差 > 1ms，应被跳过
        assert!(cookies.is_empty(), "过期会话不应被加载");
    }

    #[tokio::test]
    async fn cf_set_replaces_same_name() {
        let (jar, _dir) = make_jar();
        let c1 = Cookie {
            name: "x".into(),
            value: "v1".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        let c2 = Cookie {
            name: "x".into(),
            value: "v2".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(c1).await;
        jar.set(c2).await;

        let url = make_url("https://example.com/");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1, "同名 cookie 应被替换");
        assert_eq!(cookies[0].value, "v2");
    }
}
