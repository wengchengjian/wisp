//! CF 会话 cookie jar：moka 内存热缓存 + 本地 JSON 文件持久化。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use moka::sync::Cache;
use url::Url;

use super::session::CfSession;
use crate::cookie::{Cookie, CookieJar};

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
        Cookie::from_cdp_value(v, default_domain)
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

    async fn set_batch(&self, cookies: Vec<Cookie>) {
        if cookies.is_empty() {
            return;
        }
        // 按 domain 分组，合并到现有 session（复用 set 的合并逻辑）
        let mut by_domain: HashMap<String, CfSession> = HashMap::new();
        for cookie in cookies {
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
            let session = by_domain.entry(domain.clone()).or_insert_with(|| {
                self.get_session(&domain).unwrap_or_else(|| CfSession {
                    cookies: Vec::new(),
                    ua: String::new(),
                    saved_at: chrono::Utc::now().timestamp(),
                })
            });
            session
                .cookies
                .retain(|c| c.get("name").and_then(|n| n.as_str()) != Some(&cookie.name));
            session.cookies.push(cookie_json);
            session.saved_at = chrono::Utc::now().timestamp();
        }
        // 内存批量更新 + 单次文件持久化（避免逐 cookie 全量序列化）
        for (domain, session) in by_domain {
            self.mem.insert(domain, session);
        }
        self.save_to_file();
    }

    async fn clear(&self, url: &Url) {
        if let Some(domain) = url.host_str() {
            self.mem.invalidate(domain);
            self.save_to_file();
        }
    }

    async fn ua(&self, url: &Url) -> Option<String> {
        let domain = url.host_str()?;
        let ua = self.get_session(domain)?.ua;
        if ua.is_empty() { None } else { Some(ua) }
    }

    async fn set_session_ua(&self, domain: &str, ua: Option<&str>) {
        let mut session = self.get_session(domain).unwrap_or_else(|| CfSession {
            cookies: Vec::new(),
            ua: String::new(),
            saved_at: chrono::Utc::now().timestamp(),
        });
        match ua {
            Some(ua) => session.ua = ua.to_string(),
            None => session.ua.clear(),
        }
        session.saved_at = chrono::Utc::now().timestamp();
        self.insert_session(domain.to_string(), session);
    }
}
