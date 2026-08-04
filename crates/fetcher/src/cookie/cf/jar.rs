//! CF 会话 cookie jar：moka 内存热缓存 + 本地 JSON 文件持久化。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use moka::sync::Cache;
use url::Url;

use super::session::CfSession;
use crate::cookie::{Cookie, CookieJar};

/// 写盘去抖窗口：合并该窗口内的多次变更，避免高频全量序列化/写盘。
const FLUSH_DEBOUNCE: Duration = Duration::from_millis(200);

/// CF 会话 cookie jar：moka 内存热缓存 + 本地 JSON 文件持久化。
///
/// - 读取：moka 优先（TTL 由 moka 管理）
/// - 写入：moka + 文件双写（write-through，落盘去抖合并）
/// - 启动：从文件加载未过期条目到 moka
pub struct CfCookieJar {
    inner: Arc<Inner>,
}

/// 共享内部状态：去抖标记与底层存储放在同一 `Arc` 下，便于后台任务持引用。
struct Inner {
    mem: Cache<String, CfSession>,
    file_path: PathBuf,
    /// 存在尚未落盘的变更（去抖合并依据）。
    dirty: AtomicBool,
    /// 是否已有 flush 任务在途，防止并发重复落盘。
    flushing: AtomicBool,
}

impl CfCookieJar {
    /// 创建 jar：从文件加载未过期条目到 moka。
    #[must_use]
    pub fn new(data_dir: &Path, ttl: Duration) -> Self {
        let file_path = data_dir.join("cf_sessions.json");
        let mem: Cache<String, CfSession> =
            Cache::builder().time_to_live(ttl).max_capacity(64).build();
        let inner = Arc::new(Inner {
            mem,
            file_path,
            dirty: AtomicBool::new(false),
            flushing: AtomicBool::new(false),
        });
        let cache = Self { inner };
        cache.load_from_file(ttl);
        cache
    }

    /// 读取 CF 会话（moka 优先，启动时已批量加载文件）。
    #[must_use]
    pub fn get_session(&self, domain: &str) -> Option<CfSession> {
        self.inner.mem.get(domain)
    }

    /// 写入 CF 会话（moka + 文件双写，落盘去抖合并）。
    pub fn insert_session(&self, domain: String, session: CfSession) {
        self.inner.mem.insert(domain, session);
        self.schedule_flush();
    }

    /// 立即同步落盘当前快照。
    ///
    /// 生产路径由 `schedule_flush` 去抖异步落盘；此处供测试/进程退出前保证持久化。
    pub fn flush(&self) {
        self.inner.flush_blocking();
    }

    /// 文件加载：启动时调用，跳过过期条目。
    fn load_from_file(&self, ttl: Duration) {
        let content = match std::fs::read_to_string(&self.inner.file_path) {
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
                self.inner.mem.insert(domain, session);
                loaded += 1;
            }
        }
        if loaded > 0 {
            tracing::info!("CF 会话缓存: 从文件恢复 {loaded} 个域名的会话");
        }
    }

    /// 去抖落盘：合并短时间内的多次变更，避免高频全量写文件。
    ///
    /// - 有 tokio runtime：spawn 去抖任务，合并 `FLUSH_DEBOUNCE` 窗口内的并发写入，
    ///   用异步 IO 落盘，不阻塞 executor。
    /// - 无 runtime（同步上下文）：退化为同步落盘，保证持久化语义不被破坏。
    fn schedule_flush(&self) {
        self.inner.dirty.store(true, Ordering::Release);
        if self.inner.flushing.swap(true, Ordering::AcqRel) {
            return; // 已有 flush 在途，本次变更由它一并覆盖
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let inner = Arc::clone(&self.inner);
            handle.spawn(async move {
                tokio::time::sleep(FLUSH_DEBOUNCE).await;
                loop {
                    inner.flush_once().await;
                    inner.dirty.store(false, Ordering::Release);
                    inner.flushing.store(false, Ordering::Release);
                    // 去抖窗口内又有新变更 → 再 flush 一轮
                    if inner.dirty.load(Ordering::Acquire) {
                        inner.flushing.store(true, Ordering::Release);
                        tokio::time::sleep(FLUSH_DEBOUNCE).await;
                        continue;
                    }
                    break;
                }
            });
        } else {
            self.inner.flush_blocking();
            self.inner.dirty.store(false, Ordering::Release);
            self.inner.flushing.store(false, Ordering::Release);
        }
    }

    /// 从 serde_json::Value 提取 Cookie（CDP Network.getCookies 返回格式）。
    fn value_to_cookie(v: &serde_json::Value, default_domain: &str) -> Option<Cookie> {
        Cookie::from_cdp_value(v, default_domain)
    }
}

impl Inner {
    /// 异步落盘当前 moka 快照（紧凑序列化 + 异步 IO，不阻塞 executor）。
    async fn flush_once(&self) {
        let Some(json) = self.snapshot() else {
            return;
        };
        if let Some(parent) = self.file_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(e) = tokio::fs::write(&self.file_path, json).await {
            tracing::warn!("CF 会话文件写入失败: {e}");
        }
    }

    /// 同步落盘（无 runtime 上下文时使用）。
    fn flush_blocking(&self) {
        let Some(json) = self.snapshot() else {
            return;
        };
        if let Some(parent) = self.file_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&self.file_path, json) {
            tracing::warn!("CF 会话文件写入失败: {e}");
        }
    }

    /// 序列化当前 moka 快照（紧凑格式，减少写入字节量）。
    fn snapshot(&self) -> Option<String> {
        let mut map = HashMap::new();
        for (domain, session) in &self.mem {
            map.insert(domain.to_string(), session.clone());
        }
        match serde_json::to_string(&map) {
            Ok(json) => Some(json),
            Err(e) => {
                tracing::warn!("CF 会话序列化失败: {e}");
                None
            }
        }
    }
}

#[async_trait]
impl CookieJar for CfCookieJar {
    async fn get(&self, url: &Url) -> Vec<Cookie> {
        let Some(domain) = url.host_str() else {
            return Vec::new();
        };
        // 按完整主机名匹配存在缺陷：父域 cookie 会漏。此处保持与 moka 按写入 key 查询，
        // 因为 CF 会话以「实际访问的主机」为 key 写入，读取时用同一 host 即可命中。
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
        // 内存批量更新 + 单次去抖落盘（避免逐 cookie 全量序列化）
        for (domain, session) in by_domain {
            self.inner.mem.insert(domain, session);
        }
        self.schedule_flush();
    }

    async fn clear(&self, url: &Url) {
        if let Some(domain) = url.host_str() {
            self.inner.mem.invalidate(domain);
            self.schedule_flush();
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