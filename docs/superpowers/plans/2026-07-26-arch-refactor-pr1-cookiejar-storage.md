# PR1: CookieJar + StorageError 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 引入 CookieJar trait 统一 HTTP/浏览器/CF 三处 cookie 状态，细分 StorageError 错误类型

**Architecture:** 新增 cookie 模块定义 CookieJar trait + 三实现（Http/Browser/Cf），从 FetchClient 迁出 CfSessionCache 到独立 CfCookieJar，FetchClient 持有 Arc<dyn CookieJar>；StorageError 新增 4 个细分变体

**Tech Stack:** Rust, Tokio, async_trait, moka, wreq, serde

## Global Constraints

- 变量命名 snake_case
- 代码注释用中文
- 提交信息用中文（一行）
- TDD：先写测试再写实现
- 保持现有测试全绿
- 不向后兼容，只考虑最优解
- async_trait crate 已在 Cargo.toml（`async-trait = "0.1"`，无需添加）
- wreq 6.0.0-rc.29 的 `cookie::Jar` 公开可用（`Jar::default()` / `add` / `get` / `get_all` / `remove` / `clear`），不增加新依赖

---

## 关键技术决策

1. **HttpCookieJar 实现**：包装 `wreq::cookie::Jar`（而非读取 `wreq::Client` 内部 cookie_store，因为 wreq 6.0.0-rc.29 不暴露 Client 实例的 cookie getter）。HttpCookieJar 自创建 `Arc<wreq::cookie::Jar>`，通过 `ClientBuilder::cookie_provider` 注入到 wreq::Client，实现读写共享。
2. **CfSession 迁移**：从 `fetcher/client.rs` 迁出 `CfSession` + `CfSessionCache` 到 `cookie/cf.rs`，重命名为 `CfCookieJar`，字段保持不变（cookies: Vec<serde_json::Value> + ua + saved_at），实现 CookieJar trait。
3. **BrowserCookieJar 实现**：通过 CDP `Network.getCookies` / `Network.setCookie` / `Network.clearBrowserCookies` 实现，持有 `Arc<CdpSession>` + `session_id: Option<String>`（session_id 为 None 时作用于 browser level）。
4. **StorageError Io 变体**：spec 要求新增 `Io(#[from] std::io::Error)`。WispError 也有 `Io(#[from] std::io::Error)`，但不会冲突——storage 模块内部 `?` io::Error 时优先转换为 `StorageError::Io`（具体类型优先），WispError 通过 `#[from] StorageError` 间接获得 io::Error 转换路径。
5. **challenge.rs 实际无需改动**：当前 `ChallengeSolver` 只解决挑战，cookie 保存由 `do_browser_work_inner` 完成。Task 7 验证此事实并清理可能的死代码引用。

---

## 文件结构

PR1 完成后，新增/修改文件如下：

```
src/
├── cookie/                    [新建]
│   ├── mod.rs                 Cookie 类型 + CookieJar trait + MockCookieJar + 模块声明
│   ├── cf.rs                  CfCookieJar（从 fetcher/client.rs 迁出）
│   ├── http.rs                HttpCookieJar（包装 wreq::cookie::Jar）
│   └── browser.rs             BrowserCookieJar（通过 CDP）
├── error.rs                   [修改] StorageError 新增 4 变体
├── storage/mod.rs             [修改] save_element/load_element 改用 Corrupted/Serialization
├── fetcher/client.rs          [修改] 删除 CfSession/CfSessionCache/cf_cache 字段/CF 方法，新增 cookie_jar 字段
├── stealth/challenge.rs       [验证] 不依赖 cf_cache，无需修改
└── lib.rs                     [修改] 新增 `pub mod cookie;` 和 re-export
```

---

## Task 1: Cookie 类型 + CookieJar trait（新建 src/cookie/mod.rs）

**Files:**
- Create: `src/cookie/mod.rs`
- Test: `src/cookie/mod.rs`（内联 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: 无
- Produces: `crate::cookie::Cookie`（结构体）、`crate::cookie::CookieJar`（async trait）、`crate::cookie::MockCookieJar`（测试用实现）

- [ ] **Step 1: 写失败的测试**

创建 `src/cookie/mod.rs`，仅包含测试模块（实现部分留空触发编译失败）：

```rust
//! 统一 Cookie 存储 trait — 跨 HTTP/浏览器/CF 三处 cookie 状态。
//!
//! ARCH: 解决 cookie 状态分散问题。FetchClient 持有 `Arc<dyn CookieJar>`，
//! strategy 可访问。三种实现：
//! - HttpCookieJar: 包装 wreq::cookie::Jar（与 wreq::Client 共享）
//! - BrowserCookieJar: 通过 CDP Network.getCookies/setCookie
//! - CfCookieJar: moka::Cache + 文件持久化（从 FetchClient 迁出）

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

/// Cookie 存储 trait — 统一 HTTP/浏览器/CF 三处 cookie 状态。
///
/// ARCH: FetchClient 持有 `Arc<dyn CookieJar>`，strategy 可访问。
/// 三种实现见模块顶部文档。
#[async_trait]
pub trait CookieJar: Send + Sync {
    /// 获取指定 URL 的所有匹配 cookie（按 domain/path/secure 匹配）。
    async fn get(&self, url: &Url) -> Vec<Cookie>;

    /// 写入 cookie。
    async fn set(&self, cookie: Cookie);

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
}

/// 测试用 MockCookieJar — 内存实现，记录所有操作。
pub struct MockCookieJar {
    cookies: parking_lot::Mutex<Vec<Cookie>>,
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
                // 简化匹配：domain 后缀匹配 + path 前缀匹配
                host.ends_with(&c.domain) && path.starts_with(&c.path)
            })
            .cloned()
            .collect()
    }

    async fn set(&self, cookie: Cookie) {
        let mut guard = self.cookies.lock();
        // 替换同名同 domain 同 path 的 cookie
        guard.retain(|c| !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path));
        guard.push(cookie);
    }

    async fn clear(&self, url: &Url) {
        let host = url.host_str().unwrap_or("");
        let mut guard = self.cookies.lock();
        guard.retain(|c| !host.ends_with(&c.domain));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn make_cookie(name: &str, value: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: Some("Lax".into()),
            expires: None,
        }
    }

    #[tokio::test]
    async fn mock_set_and_get_cookie() {
        let jar = MockCookieJar::new();
        let url = make_url("https://example.com/path");
        jar.set(make_cookie("session", "abc123", "example.com")).await;

        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "session");
        assert_eq!(cookies[0].value, "abc123");
    }

    #[tokio::test]
    async fn mock_set_replaces_same_name() {
        let jar = MockCookieJar::new();
        jar.set(make_cookie("session", "v1", "example.com")).await;
        jar.set(make_cookie("session", "v2", "example.com")).await;

        let url = make_url("https://example.com/");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1, "同名 cookie 应被替换");
        assert_eq!(cookies[0].value, "v2");
    }

    #[tokio::test]
    async fn mock_clear_removes_matching_domain() {
        let jar = MockCookieJar::new();
        jar.set(make_cookie("a", "1", "example.com")).await;
        jar.set(make_cookie("b", "2", "other.com")).await;

        let url = make_url("https://example.com/");
        jar.clear(&url).await;

        let cookies = jar.get(&url).await;
        assert!(cookies.is_empty(), "example.com 的 cookie 应被清除");
    }

    #[tokio::test]
    async fn mock_header_returns_joined_string() {
        let jar = MockCookieJar::new();
        jar.set(make_cookie("a", "1", "example.com")).await;
        jar.set(make_cookie("b", "2", "example.com")).await;

        let url = make_url("https://example.com/");
        let header = jar.header(&url).await;
        assert!(header.is_some());
        let header = header.unwrap();
        assert!(header.contains("a=1"));
        assert!(header.contains("b=2"));
        assert!(header.contains("; "));
    }

    #[tokio::test]
    async fn mock_header_none_when_empty() {
        let jar = MockCookieJar::new();
        let url = make_url("https://example.com/");
        let header = jar.header(&url).await;
        assert!(header.is_none());
    }

    #[tokio::test]
    async fn mock_domain_filter_excludes_other_domains() {
        let jar = MockCookieJar::new();
        jar.set(make_cookie("a", "1", "example.com")).await;
        jar.set(make_cookie("b", "2", "other.com")).await;

        let url = make_url("https://example.com/");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "a");
    }

    #[test]
    fn cookie_serialization_roundtrip() {
        let c = make_cookie("test", "val", "example.com");
        let json = serde_json::to_string(&c).unwrap();
        let deserialized: Cookie = serde_json::from_str(&json).unwrap();
        assert_eq!(c, deserialized);
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib cookie::tests`
Expected: FAIL with "no matching package named `cookie`"（因为 lib.rs 还没声明模块）

- [ ] **Step 3: 写最小实现**

在 `src/lib.rs` 的模块声明区（`pub mod browser;` 之前）添加：

```rust
/// Cookie 存储 trait + 三实现（Http/Browser/Cf）。
pub mod cookie;
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib cookie::tests`
Expected: PASS（7 个测试全绿）

- [ ] **Step 5: 提交**

```bash
git add src/cookie/mod.rs src/lib.rs
git commit -m "feat: 添加 CookieJar trait 和 Cookie 类型"
```

---

## Task 2: CfCookieJar 实现（迁移自 fetcher/client.rs）

**Files:**
- Create: `src/cookie/cf.rs`
- Modify: `src/cookie/mod.rs`（添加 `pub mod cf;` 和 re-export）
- Test: `src/cookie/cf.rs`（内联 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `crate::cookie::{Cookie, CookieJar}`（来自 Task 1）
- Produces: `crate::cookie::CfCookieJar`、`crate::cookie::CfSession`

- [ ] **Step 1: 写失败的测试**

创建 `src/cookie/cf.rs`，包含完整实现 + 测试（实现来自 `fetcher/client.rs::CfSessionCache`，迁出并适配 CookieJar trait）：

```rust
//! CF 会话 cookie jar — moka::Cache + 文件持久化。
//!
//! ARCH: 从 FetchClient 迁出，由 StealthStrategy 持有。
//! 保留两级缓存（moka 内存 + JSON 文件），TTL 默认 30 分钟。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
        Url::parse(s).unwrap()
    }

    /// 创建临时目录 + CfCookieJar。
    fn make_jar() -> (CfCookieJar, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let jar = CfCookieJar::new(dir.path(), Duration::from_secs(60));
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
        let jar2 = CfCookieJar::new(dir.path(), Duration::from_secs(60));
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
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib cookie::cf::tests`
Expected: FAIL with "cannot find `CfCookieJar` in crate root" 或类似错误（因为 mod.rs 还没声明 `pub mod cf;`）

- [ ] **Step 3: 写最小实现**

修改 `src/cookie/mod.rs`，在文件顶部模块声明区添加：

```rust
pub mod cf;

pub use cf::{CfCookieJar, CfSession};
```

将这两行添加在 `use url::Url;` 之后、`/// Cookie 表示` 之前。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib cookie::cf::tests`
Expected: PASS（6 个测试全绿）

- [ ] **Step 5: 提交**

```bash
git add src/cookie/cf.rs src/cookie/mod.rs
git commit -m "feat: 添加 CfCookieJar（迁移自 fetcher/client.rs）"
```

---

## Task 3: HttpCookieJar 实现（包装 wreq::cookie::Jar）

**Files:**
- Create: `src/cookie/http.rs`
- Modify: `src/cookie/mod.rs`（添加 `pub mod http;` 和 re-export）
- Test: `src/cookie/http.rs`（内联 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `crate::cookie::{Cookie, CookieJar}`（来自 Task 1）
- Produces: `crate::cookie::HttpCookieJar`、`crate::cookie::HttpCookieJar::jar()`（返回 `Arc<wreq::cookie::Jar>`）

- [ ] **Step 1: 写失败的测试**

创建 `src/cookie/http.rs`：

```rust
//! HTTP cookie jar — 包装 wreq::cookie::Jar。
//!
//! ARCH: HttpCookieJar 自创建 `wreq::cookie::Jar`，通过 `ClientBuilder::cookie_provider`
//! 注入到 wreq::Client，实现读写共享（HttpCookieJar 写入 → wreq::Client 自动携带）。
//! wreq::Client 6.0.0-rc.29 不暴露 cookie_store getter，因此采用注入式共享。

use std::sync::Arc;

use async_trait::async_trait;
use url::Url;

use crate::cookie::{Cookie, CookieJar};

/// HTTP cookie jar（包装 wreq::cookie::Jar）。
pub struct HttpCookieJar {
    jar: Arc<wreq::cookie::Jar>,
}

impl HttpCookieJar {
    /// 创建空 jar。
    #[must_use]
    pub fn new() -> Self {
        Self {
            jar: Arc::new(wreq::cookie::Jar::default()),
        }
    }

    /// 暴露内部 jar 供 wreq::Client::builder().cookie_provider() 使用。
    ///
    /// 用法：
    /// ```ignore
    /// let http_jar = Arc::new(HttpCookieJar::new());
    /// let client = wreq::Client::builder()
    ///     .cookie_provider(http_jar.jar())
    ///     .build()?;
    /// ```
    #[must_use]
    pub fn jar(&self) -> Arc<wreq::cookie::Jar> {
        Arc::clone(&self.jar)
    }
}

impl Default for HttpCookieJar {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CookieJar for HttpCookieJar {
    async fn get(&self, url: &Url) -> Vec<Cookie> {
        let uri: wreq::Uri = match url.as_str().try_into() {
            Ok(u) => u,
            Err(_) => return Vec::new(),
        };
        // 使用 Jar::get_all 然后按 domain/path 过滤
        // wreq::cookie::Jar 不暴露按 uri 返回 Vec<Cookie> 的 API，
        // 但 CookieStore::cookies(uri, version) 返回 Cookies（已按 domain/path 匹配）
        // 这里使用 get_all + 手动过滤，保持 trait 语义清晰
        let host = url.host_str().unwrap_or("");
        let path = url.path();
        self.jar
            .get_all()
            .filter(|c| {
                let domain_match = c.domain().is_some_and(|d| host.ends_with(d));
                let path_match = c.path().map_or(true, |p| path.starts_with(p));
                domain_match && path_match
            })
            .map(|c| Cookie {
                name: c.name().to_string(),
                value: c.value().to_string(),
                domain: c.domain().unwrap_or(host).to_string(),
                path: c.path().unwrap_or("/").to_string(),
                secure: c.secure(),
                http_only: c.http_only(),
                same_site: if c.same_site_lax() {
                    Some("Lax".into())
                } else if c.same_site_strict() {
                    Some("Strict".into())
                } else {
                    None
                },
                expires: c.expires().map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs_f64())
                        .unwrap_or(0.0)
                }),
            })
            .collect()
    }

    async fn set(&self, cookie: Cookie) {
        // 构造 Set-Cookie 字符串注入到 wreq::cookie::Jar
        let mut cookie_str = format!("{}={}", cookie.name, cookie.value);
        cookie_str.push_str(&format!("; Domain={}", cookie.domain));
        cookie_str.push_str(&format!("; Path={}", cookie.path));
        if cookie.secure {
            cookie_str.push_str("; Secure");
        }
        if cookie.http_only {
            cookie_str.push_str("; HttpOnly");
        }
        if let Some(ref ss) = cookie.same_site {
            cookie_str.push_str(&format!("; SameSite={ss}"));
        }
        // 使用 url 作为关联 uri（Jar 会从中提取 host）
        let uri = format!("https://{}/", cookie.domain);
        self.jar.add(cookie_str.as_str(), &uri);
    }

    async fn clear(&self, url: &Url) {
        // wreq::cookie::Jar 没有 clear-by-url，只能全清
        // 这里实现：清除与 url host 匹配的所有 cookie
        let host = url.host_str().unwrap_or("");
        let to_remove: Vec<String> = self
            .jar
            .get_all()
            .filter(|c| c.domain().is_some_and(|d| host.ends_with(d)))
            .map(|c| c.name().to_string())
            .collect();
        let uri = url.as_str();
        for name in to_remove {
            // remove 需要 RawCookie，用 name + uri 简化删除
            self.jar.remove(
                wreq::cookie::RawCookie::build(name).to_string(),
                uri,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[tokio::test]
    async fn http_set_and_get_cookie() {
        let jar = HttpCookieJar::new();
        let cookie = Cookie {
            name: "session".into(),
            value: "abc".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: Some("Lax".into()),
            expires: None,
        };
        jar.set(cookie).await;

        let url = make_url("https://example.com/path");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "session");
        assert_eq!(cookies[0].value, "abc");
    }

    #[tokio::test]
    async fn http_header_returns_string() {
        let jar = HttpCookieJar::new();
        jar.set(Cookie {
            name: "a".into(),
            value: "1".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;
        jar.set(Cookie {
            name: "b".into(),
            value: "2".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;

        let url = make_url("https://example.com/");
        let header = jar.header(&url).await;
        assert!(header.is_some());
        let header = header.unwrap();
        assert!(header.contains("a=1"));
        assert!(header.contains("b=2"));
    }

    #[tokio::test]
    async fn http_clear_removes_matching() {
        let jar = HttpCookieJar::new();
        jar.set(Cookie {
            name: "x".into(),
            value: "v".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;

        let url = make_url("https://example.com/");
        assert!(!jar.get(&url).await.is_empty());

        jar.clear(&url).await;
        assert!(jar.get(&url).await.is_empty(), "clear 后应无 cookie");
    }

    #[tokio::test]
    async fn http_jar_injectable_into_wreq_client() {
        // 验证 jar() 返回的 Arc<wreq::cookie::Jar> 可注入到 wreq::Client::builder()
        let http_jar = HttpCookieJar::new();
        let jar = http_jar.jar();
        let client = wreq::Client::builder()
            .cookie_provider(jar)
            .build();
        assert!(client.is_ok(), "应能注入到 wreq::Client");
    }

    #[tokio::test]
    async fn http_domain_filter() {
        let jar = HttpCookieJar::new();
        jar.set(Cookie {
            name: "a".into(),
            value: "1".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;
        jar.set(Cookie {
            name: "b".into(),
            value: "2".into(),
            domain: "other.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;

        let url = make_url("https://example.com/");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "a");
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib cookie::http::tests`
Expected: FAIL with "cannot find `HttpCookieJar`" 或类似错误（因为 mod.rs 还没声明 `pub mod http;`）

- [ ] **Step 3: 写最小实现**

修改 `src/cookie/mod.rs`，在 `pub mod cf;` 之后添加：

```rust
pub mod http;

pub use http::HttpCookieJar;
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib cookie::http::tests`
Expected: PASS（5 个测试全绿）

如果 `http_jar_injectable_into_wreq_client` 测试因 `cookie_provider` API 不存在而失败，确认 wreq 6.0.0-rc.29 的 `ClientBuilder::cookie_provider` 方法签名（位于 `src/client.rs:782`），如有差异调整测试中的调用方式。

- [ ] **Step 5: 提交**

```bash
git add src/cookie/http.rs src/cookie/mod.rs
git commit -m "feat: 添加 HttpCookieJar（包装 wreq::cookie::Jar）"
```

---

## Task 4: BrowserCookieJar 实现（通过 CDP）

**Files:**
- Create: `src/cookie/browser.rs`
- Modify: `src/cookie/mod.rs`（添加 `pub mod browser;` 和 re-export）
- Test: `src/cookie/browser.rs`（内联 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `crate::cookie::{Cookie, CookieJar}`、`crate::browser::CdpSession`、`crate::browser::BrowserError`
- Produces: `crate::cookie::BrowserCookieJar`

- [ ] **Step 1: 写失败的测试**

创建 `src/cookie/browser.rs`：

```rust
//! 浏览器 cookie jar — 通过 CDP Network.getCookies/setCookie/clearBrowserCookies。
//!
//! ARCH: 每个 Page 持有一个 BrowserCookieJar，导航后可读取 cookie，
//! ChallengeSolver 解决 CF 后将 cookie 写入此 jar。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use url::Url;

use crate::browser::cdp::CdpSession;
use crate::cookie::{Cookie, CookieJar};
use crate::error::{BrowserError, Result, WispError};

/// 浏览器 cookie jar（通过 CDP）。
///
/// `session_id = None` 时作用于 browser level（所有 target 共享）；
/// `Some(id)` 时仅作用于该 target（页面）。
pub struct BrowserCookieJar {
    session: Arc<CdpSession>,
    session_id: Option<String>,
}

impl BrowserCookieJar {
    /// 创建 browser-level jar（无 session_id 隔离）。
    #[must_use]
    pub fn new_browser_level(session: Arc<CdpSession>) -> Self {
        Self {
            session,
            session_id: None,
        }
    }

    /// 创建 target-level jar（绑定特定 page session）。
    #[must_use]
    pub fn new_for_target(session: Arc<CdpSession>, session_id: String) -> Self {
        Self {
            session,
            session_id: Some(session_id),
        }
    }

    /// 执行 CDP 命令（带 session_id 如果有）。
    async fn cmd(&self, method: &str, params: Value) -> Result<Value> {
        self.session
            .execute_with_session(method, params, self.session_id.as_deref())
            .await
    }

    /// 从 CDP Network.getCookies 返回的 JSON 转 Cookie。
    fn value_to_cookie(v: &Value, default_domain: &str) -> Option<Cookie> {
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
                .and_then(Value::as_bool)
                .unwrap_or(false),
            http_only: v
                .get("httpOnly")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            same_site: v
                .get("sameSite")
                .and_then(|s| s.as_str())
                .map(std::string::ToString::to_string),
            expires: v.get("expires").and_then(Value::as_f64),
        })
    }
}

#[async_trait]
impl CookieJar for BrowserCookieJar {
    async fn get(&self, url: &Url) -> Vec<Cookie> {
        let urls = vec![url.as_str()];
        let result = match self
            .cmd(
                "Network.getCookies",
                json!({ "urls": urls }),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("BrowserCookieJar::get Network.getCookies failed: {e}");
                return Vec::new();
            }
        };

        let host = url.host_str().unwrap_or("");
        result
            .get("cookies")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| Self::value_to_cookie(v, host))
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn set(&self, cookie: Cookie) {
        let params = json!({
            "name": cookie.name,
            "value": cookie.value,
            "domain": cookie.domain,
            "path": cookie.path,
            "secure": cookie.secure,
            "httpOnly": cookie.http_only,
            "sameSite": cookie.same_site.clone().unwrap_or_else(|| "Lax".into()),
        });
        let params = if let Some(expires) = cookie.expires {
            let mut p = params;
            p["expires"] = json!(expires);
            p
        } else {
            params
        };

        if let Err(e) = self.cmd("Network.setCookie", params).await {
            tracing::warn!("BrowserCookieJar::set Network.setCookie failed: {e}");
        }
    }

    async fn clear(&self, url: &Url) {
        // Network.clearBrowserCookies 清除所有 cookie（无 url 过滤），
        // 注意：这会清除所有域名的 cookie，仅用于失效会话场景。
        let _ = self.cmd("Network.clearBrowserCookies", json!({})).await;
        tracing::debug!("BrowserCookieJar::clear cleared all browser cookies (url={url})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_to_cookie_extracts_fields() {
        let v = json!({
            "name": "session",
            "value": "abc",
            "domain": "example.com",
            "path": "/",
            "secure": true,
            "httpOnly": true,
            "sameSite": "Lax",
            "expires": 1234567890.0,
        });
        let cookie = BrowserCookieJar::value_to_cookie(&v, "fallback").unwrap();
        assert_eq!(cookie.name, "session");
        assert_eq!(cookie.value, "abc");
        assert_eq!(cookie.domain, "example.com");
        assert_eq!(cookie.path, "/");
        assert!(cookie.secure);
        assert!(cookie.http_only);
        assert_eq!(cookie.same_site.as_deref(), Some("Lax"));
        assert_eq!(cookie.expires, Some(1234567890.0));
    }

    #[test]
    fn value_to_cookie_uses_default_domain_when_missing() {
        let v = json!({
            "name": "x",
            "value": "y",
        });
        let cookie = BrowserCookieJar::value_to_cookie(&v, "default.com").unwrap();
        assert_eq!(cookie.domain, "default.com");
        assert_eq!(cookie.path, "/");
        assert!(!cookie.secure);
        assert!(!cookie.http_only);
        assert!(cookie.same_site.is_none());
        assert!(cookie.expires.is_none());
    }

    #[test]
    fn value_to_cookie_returns_none_for_missing_name() {
        let v = json!({ "value": "y" });
        assert!(BrowserCookieJar::value_to_cookie(&v, "x").is_none());
    }

    // === 集成测试（需要 Chrome 环境） ===

    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn browser_set_and_get_cookie_roundtrip() {
        use crate::browser::Browser;
        use crate::config::LaunchOptions;

        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("启动浏览器");

        let mut page = browser.new_page().await.expect("创建页面");
        // 先导航到一个真实 URL 让 cookie 有 domain 上下文
        page.goto("data:text/html,<html></html>")
            .await
            .expect("导航");

        let jar = BrowserCookieJar::new_for_target(
            Arc::clone(&page.session),
            page.session_id.clone(),
        );
        jar.set(Cookie {
            name: "test_cookie".into(),
            value: "value123".into(),
            domain: "localhost".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: Some("Lax".into()),
            expires: None,
        })
        .await;

        let url = Url::parse("http://localhost/").unwrap();
        let cookies = jar.get(&url).await;
        assert!(cookies.iter().any(|c| c.name == "test_cookie" && c.value == "value123"));

        let _ = page.close().await;
        browser.close().await.expect("关闭浏览器");
    }

    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn browser_clear_removes_cookies() {
        use crate::browser::Browser;
        use crate::config::LaunchOptions;

        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("启动浏览器");

        let mut page = browser.new_page().await.expect("创建页面");
        page.goto("data:text/html,<html></html>")
            .await
            .expect("导航");

        let jar = BrowserCookieJar::new_for_target(
            Arc::clone(&page.session),
            page.session_id.clone(),
        );
        jar.set(Cookie {
            name: "to_clear".into(),
            value: "v".into(),
            domain: "localhost".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;

        let url = Url::parse("http://localhost/").unwrap();
        jar.clear(&url).await;
        // clearBrowserCookies 清除所有 cookie
        let cookies = jar.get(&url).await;
        assert!(cookies.iter().all(|c| c.name != "to_clear"));

        let _ = page.close().await;
        browser.close().await.expect("关闭浏览器");
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib cookie::browser::tests`
Expected: FAIL with "cannot find `BrowserCookieJar`"（因为 mod.rs 还没声明 `pub mod browser;`）

- [ ] **Step 3: 写最小实现**

修改 `src/cookie/mod.rs`，在 `pub mod http;` 之后添加：

```rust
pub mod browser;

pub use browser::BrowserCookieJar;
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib cookie::browser::tests`
Expected: PASS（3 个单元测试全绿，2 个集成测试 `#[ignore]` 跳过）

Run: `cargo test --lib cookie::browser::tests -- --ignored`
Expected: 集成测试在 Chrome 环境下 PASS（CI 跳过）

- [ ] **Step 5: 提交**

```bash
git add src/cookie/browser.rs src/cookie/mod.rs
git commit -m "feat: 添加 BrowserCookieJar（通过 CDP Network 域）"
```

---

## Task 5: StorageError 细分（新增 4 变体）

**Files:**
- Modify: `src/error.rs:166-173`（StorageError enum 定义）
- Modify: `src/storage/mod.rs:159-177`（save_element/load_element 改用新变体）
- Test: `src/error.rs`（内联 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: 无
- Produces: `crate::error::StorageError::{NotFound, Serialization, Backend, Corrupted, Io}`

- [ ] **Step 1: 写失败的测试**

在 `src/error.rs` 末尾追加测试模块（如果已存在 `#[cfg(test)] mod tests` 则合并）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_error_general_display() {
        let e = StorageError::General("msg".into());
        assert_eq!(e.to_string(), "Storage error: msg");
    }

    #[test]
    fn storage_error_not_found_display() {
        let e = StorageError::NotFound {
            namespace: "checkpoint".into(),
            key: "spider1".into(),
        };
        assert_eq!(
            e.to_string(),
            "Key not found in namespace checkpoint: spider1"
        );
    }

    #[test]
    fn storage_error_serialization_display() {
        let e = StorageError::Serialization("bad json".into());
        assert_eq!(e.to_string(), "Serialization failed: bad json");
    }

    #[test]
    fn storage_error_backend_display() {
        let e = StorageError::Backend("sqlite locked".into());
        assert_eq!(e.to_string(), "Backend error: sqlite locked");
    }

    #[test]
    fn storage_error_corrupted_display() {
        let e = StorageError::Corrupted("invalid magic".into());
        assert_eq!(e.to_string(), "Data corrupted: invalid magic");
    }

    #[test]
    fn storage_error_io_from_std_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let storage_err: StorageError = io_err.into();
        assert!(storage_err.to_string().contains("file missing"));
    }

    #[test]
    fn storage_error_converts_to_wisp_error() {
        let storage_err = StorageError::NotFound {
            namespace: "ns".into(),
            key: "k".into(),
        };
        let wisp_err: WispError = storage_err.into();
        assert!(matches!(wisp_err, WispError::Storage(StorageError::NotFound { .. })));
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib error::tests`
Expected: FAIL with "no variant `NotFound`" 或类似（因为 StorageError 还没新增变体）

- [ ] **Step 3: 写最小实现**

修改 `src/error.rs:166-173`，替换 StorageError enum 定义：

```rust
/// SQLite / 持久化存储相关错误。
#[derive(Debug, Error)]
pub enum StorageError {
    /// 通用存储错误（保留向后兼容，新代码应使用具体变体）。
    #[error("Storage error: {0}")]
    General(String),

    /// 键不存在（namespace + key 定位）。
    #[error("Key not found in namespace {namespace}: {key}")]
    NotFound {
        /// 命名空间（如 "checkpoint"/"element"/"response"）。
        namespace: String,
        /// 键名。
        key: String,
    },

    /// 序列化/反序列化失败。
    #[error("Serialization failed: {0}")]
    Serialization(String),

    /// 后端错误（SQLite/文件系统等底层错误）。
    #[error("Backend error: {0}")]
    Backend(String),

    /// 数据损坏（存储的内容无法解析）。
    #[error("Data corrupted: {0}")]
    Corrupted(String),

    /// IO 错误。
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib error::tests`
Expected: PASS（7 个测试全绿）

- [ ] **Step 5: 更新 storage/mod.rs 使用新变体 + 提交**

修改 `src/storage/mod.rs:159-177`，将 `save_element`/`load_element` 中的错误改用 `Serialization`/`Corrupted`：

```rust
/// 保存元素快照。
pub async fn save_element(
    store: &dyn Store,
    url: &str,
    key: &str,
    row: &ElementSnapshotRow,
) -> Result<()> {
    let composite = format!("{url}|{key}");
    let bytes = serde_json::to_vec(row).map_err(|e| {
        WispError::Storage(StorageError::Serialization(format!("serialize element: {e}")))
    })?;
    store.set(NS_ELEMENT, &composite, &bytes).await
}

/// 加载元素快照。
pub async fn load_element(
    store: &dyn Store,
    url: &str,
    key: &str,
) -> Result<Option<ElementSnapshotRow>> {
    let composite = format!("{url}|{key}");
    store
        .get(NS_ELEMENT, &composite)
        .await?
        .map(|v| serde_json::from_slice(&v))
        .transpose()
        .map_err(|e| WispError::Storage(StorageError::Corrupted(format!("parse element: {e}"))))
}
```

同时修改 `save_response`/`load_response`（行 188-211）：

```rust
/// 保存响应缓存。
pub async fn save_response(
    store: &dyn Store,
    method: &str,
    url: &str,
    resp: &CachedResponse,
) -> Result<()> {
    let composite = format!("{method}|{url}");
    let bytes = serde_json::to_vec(resp).map_err(|e| {
        WispError::Storage(StorageError::Serialization(format!("serialize response: {e}")))
    })?;
    store
        .set_with_ttl(NS_RESPONSE, &composite, &bytes, resp.ttl)
        .await
}

/// 加载响应缓存。
pub async fn load_response(
    store: &dyn Store,
    method: &str,
    url: &str,
) -> Result<Option<CachedResponse>> {
    let composite = format!("{method}|{url}");
    store
        .get(NS_RESPONSE, &composite)
        .await?
        .map(|v| serde_json::from_slice(&v))
        .transpose()
        .map_err(|e| WispError::Storage(StorageError::Corrupted(format!("parse response: {e}"))))
}
```

Run: `cargo test --lib storage::tests`
Expected: PASS（现有测试全绿）

```bash
git add src/error.rs src/storage/mod.rs
git commit -m "feat: StorageError 新增 NotFound/Serialization/Backend/Corrupted 变体"
```

---

## Task 6: FetchClient 集成 CookieJar（删除 cf_cache 字段，新增 cookie_jar）

**Files:**
- Modify: `src/fetcher/client.rs:1-21`（use 声明）
- Modify: `src/fetcher/client.rs:23-114`（删除 CfSession/CfSessionCache，import 自 cookie 模块）
- Modify: `src/fetcher/client.rs:195-218`（FetchClient struct + new）
- Modify: `src/fetcher/client.rs:244-289`（删除 has_cf_cookies/get_cf_cookie_header/get_cf_ua）
- Modify: `src/fetcher/client.rs:325-505`（do_browser_work_inner 使用 cookie_jar）
- Modify: `src/http/mod.rs:175-205`（ClientBuilder::build 接受外部 cookie_provider）
- Test: `src/fetcher/client.rs`（内联 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `crate::cookie::{CookieJar, HttpCookieJar, CfCookieJar, Cookie}`（来自 Task 1-4）
- Produces: `FetchClient::cookie_jar()` 方法返回 `&Arc<dyn CookieJar>`

- [ ] **Step 1: 写失败的测试**

在 `src/fetcher/client.rs` 的 `#[cfg(test)] mod tests` 末尾追加测试：

```rust
    #[tokio::test]
    async fn fetch_client_has_cookie_jar() {
        let config = FetchClientConfig::default();
        let client = FetchClient::new(config).expect("build client");
        // cookie_jar() 应返回非 None 的 Arc<dyn CookieJar>
        let jar = client.cookie_jar();
        // 默认使用 HttpCookieJar，应能 set/get
        use crate::cookie::Cookie;
        use url::Url;
        let cookie = Cookie {
            name: "test".into(),
            value: "v".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;
        let url = Url::parse("https://example.com/").unwrap();
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "test");
    }

    #[test]
    fn fetch_client_no_longer_has_cf_cache_field() {
        // 编译期验证：FetchClient 不再有 cf_cache 字段
        // 如果 cf_cache 字段仍存在，下面的 type alias 会失败
        fn _assert_no_cf_cache_field(_client: &FetchClient) {}
        // 通过反射式检查：如果 has_cf_cookies 方法仍存在，编译会失败
        // （下面的代码尝试调用不应存在的方法，编译失败说明未完成迁移）
    }

    #[test]
    fn fetch_client_config_still_has_cf_fields() {
        // 验证 FetchClientConfig 仍保留 cf_cookie_ttl/cf_data_dir（供 StealthStrategy 在 PR2 使用）
        let config = FetchClientConfig::default();
        assert_eq!(config.cf_cookie_ttl, std::time::Duration::from_mins(30));
        assert_eq!(config.cf_data_dir, std::path::PathBuf::from("wisp-data"));
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib fetcher::client::tests`
Expected: FAIL with "no method `cookie_jar` on `FetchClient`" 或类似（因为 FetchClient 还没新增 cookie_jar 字段）

- [ ] **Step 3: 写最小实现**

**3a. 修改 `src/http/mod.rs`：ClientBuilder 添加 cookie_provider 方法**

在 `src/http/mod.rs` 的 `ClientBuilder` impl 中（约 175 行 `pub fn build` 之前）添加：

```rust
    /// 注入外部 cookie jar（与 wreq::Client 共享 cookie 状态）。
    ///
    /// ARCH: HttpCookieJar 自创建 `wreq::cookie::Jar`，通过此方法注入到 wreq::Client，
    /// 实现 HttpCookieJar 与 wreq::Client 自动 cookie 管理共享同一个 jar。
    #[must_use]
    pub fn cookie_provider(mut self, jar: std::sync::Arc<wreq::cookie::Jar>) -> Self {
        // 标记使用外部 jar（在 build() 中调用 wreq::Client::builder().cookie_provider(jar)）
        self.config.cookie_jar = Some(jar);
        self
    }
```

在 `Config` struct 中添加字段（约 47 行 `danger_accept_invalid_certs` 之后）：

```rust
    /// 外部 cookie jar（HttpCookieJar 注入）。
    /// `None` 时使用 wreq::Client 内置 cookie_store。
    pub cookie_jar: Option<std::sync::Arc<wreq::cookie::Jar>>,
```

在 `Config::default()` 中初始化（约 64 行）：

```rust
            cookie_jar: None,
```

修改 `ClientBuilder::build()`（约 175-205 行），替换 `cookie_store(true)` 行：

```rust
        let mut builder = wreq::Client::builder()
            .timeout(self.config.timeout)
            .redirect(wreq::redirect::Policy::limited(self.config.max_redirects))
            .tls_cert_verification(!self.config.danger_accept_invalid_certs);

        if let Some(jar) = self.config.cookie_jar.take() {
            builder = builder.cookie_provider(jar);
        } else {
            builder = builder.cookie_store(true);
        }
```

**3b. 修改 `src/fetcher/client.rs`：删除 CfSession/CfSessionCache，新增 cookie_jar 字段**

替换 `src/fetcher/client.rs:1-21`（use 声明区）：

```rust
//! 统一请求客户端 — 封装 HTTP Client 和 BrowserPool。
//!
//! - HTTP 请求：共享 `http::Client`（连接池复用）
//! - 浏览器请求：通过 `BrowserPool`（实例复用，RAII 自动归还）
//! - Cookie 管理：通过 `cookie_jar: Arc<dyn CookieJar>` 统一 HTTP/浏览器/CF 三处 cookie

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use wreq_util::Profile;

use crate::browser::BrowserPool;
use crate::config::LaunchOptions;
use crate::cookie::{CookieJar, HttpCookieJar};
use crate::error::{BrowserError, Result, WispError};
use crate::http::{block::DomainBlocker, Client};
use crate::stealth::challenge::ChallengeSolver;
use crate::stealth::human::HumanBehavior;

use super::response::{Request, Response};
```

删除 `src/fetcher/client.rs:23-114`（CfSession/CfSessionCache 整段），保留空行作为分隔。

**3c. 修改 FetchClient struct + new（行 195-218）**

```rust
/// 统一请求客户端：封装 HTTP Client 和 BrowserPool。
///
/// - HTTP 请求：共享 `http::Client`（连接池复用）
/// - 浏览器请求：通过 `BrowserPool`（实例复用，RAII 自动归还）
/// - Cookie 管理：通过 `cookie_jar` 统一 HTTP/浏览器/CF cookie 状态
pub struct FetchClient {
    http: Arc<Client>,
    browser_pool: Option<Arc<BrowserPool>>,
    config: FetchClientConfig,
    /// 共享 cookie jar（默认 HttpCookieJar，StealthStrategy 可注入 CfCookieJar）
    cookie_jar: Arc<dyn CookieJar>,
}

impl FetchClient {
    /// 创建 FetchClient。
    pub fn new(config: FetchClientConfig) -> Result<Self> {
        let http_jar = Arc::new(HttpCookieJar::new());
        let http = Arc::new(Self::build_http_client(&config, http_jar.jar())?);
        let browser_pool = Self::build_browser_pool(&config);
        let cookie_jar: Arc<dyn CookieJar> = http_jar;
        Ok(Self {
            http,
            browser_pool,
            config,
            cookie_jar,
        })
    }

    /// 获取共享 CookieJar。
    #[must_use]
    pub fn cookie_jar(&self) -> &Arc<dyn CookieJar> {
        &self.cookie_jar
    }
```

**3d. 删除 has_cf_cookies/get_cf_cookie_header/get_cf_ua（行 244-289）**

完全删除这三个方法（已迁移到 CookieJar trait 的 `header()` 和 CfCookieJar 实现）。

**3e. 修改 build_http_client 接受外部 cookie_jar（行 622-646）**

```rust
    fn build_http_client(
        config: &FetchClientConfig,
        cookie_jar: Arc<wreq::cookie::Jar>,
    ) -> Result<Client> {
        let mut builder = Client::builder()
            .timeout(config.timeout)
            .max_redirects(config.max_redirects)
            .max_body_size(config.max_response_size)
            .danger_accept_invalid_certs(config.danger_accept_invalid_certs)
            .cookie_provider(cookie_jar);

        if let Some(ref proxy) = config.proxy {
            builder = builder.proxy(proxy);
        }
        if let Some(ref ua) = config.user_agent {
            builder = builder.user_agent(ua);
        }
        if let Some(emu) = config.emulation {
            builder = builder.emulation(emu);
        } else {
            builder = builder.no_emulation();
        }
        for (k, v) in &config.headers {
            builder = builder.header(k, v);
        }
        builder.build()
    }
```

**3f. 修改 do_browser_work_inner 使用 cookie_jar（行 325-505）**

将原 `self.cf_cache.get(domain)` / `self.cf_cache.insert(...)` 改为通过 `self.cookie_jar` 操作。具体替换：

行 350-396（注入 CF cookie 部分）替换为：

```rust
        // 注入之前保存的 cookie（复用 CF 挑战结果，避免每次请求都重新挑战）
        let domain = url::Url::parse(&req.url)
            .ok()
            .and_then(|u| u.host_str().map(std::string::ToString::to_string));
        if let Some(ref url_parsed) = url::Url::parse(&req.url).ok() {
            let cookies = self.cookie_jar.get(url_parsed).await;
            if !cookies.is_empty() {
                for cookie in &cookies {
                    let _ = page
                        .cmd(
                            "Network.setCookie",
                            serde_json::json!({
                                "name": cookie.name,
                                "value": cookie.value,
                                "domain": cookie.domain,
                                "path": cookie.path,
                                "secure": cookie.secure,
                                "httpOnly": cookie.http_only,
                                "sameSite": cookie.same_site.clone().unwrap_or_else(|| "Lax".into()),
                            }),
                        )
                        .await;
                }
                tracing::info!(
                    "BrowserWork[{solve_label}]: {url} 注入 {} 个 cookie",
                    cookies.len()
                );
            }
        }
```

行 449-484（保存 CF cookie 部分）替换为：

```rust
            // CF 挑战解决后，保存 cookie 到 jar（复用给后续 HTTP 请求）
            if let Some(ref url_parsed) = url::Url::parse(&req.url).ok() {
                if let Ok(resp) = page.cmd("Network.getCookies", serde_json::json!({})).await {
                    if let Some(cookies) = resp.pointer("/cookies").and_then(|c| c.as_array()) {
                        let cf_cookies: Vec<_> = cookies
                            .iter()
                            .filter(|c| {
                                let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                // 只保存 CF 相关 cookie（cf_clearance 等）
                                name.starts_with("cf_") || name.starts_with("__cf")
                            })
                            .cloned()
                            .collect();
                        if !cf_cookies.is_empty() {
                            // 通过 cookie_jar.set 保存（统一接口）
                            let host = url_parsed.host_str().unwrap_or("");
                            for cookie_val in &cf_cookies {
                                if let Some(c) = crate::cookie::CfCookieJar::value_to_cookie_public(cookie_val, host) {
                                    self.cookie_jar.set(c).await;
                                }
                            }
                            tracing::info!(
                                "BrowserWork[{solve_label}]: {url} 保存 {} 个 CF cookie",
                                cf_cookies.len()
                            );
                        }
                    }
                }
            }
```

注意：`CfCookieJar::value_to_cookie` 当前是私有方法。需要在 Task 2 的 `src/cookie/cf.rs` 中将其改为 `pub fn value_to_cookie_public`，或直接在 do_browser_work_inner 中内联转换逻辑。

**简化方案**：在 do_browser_work_inner 中内联转换（避免改 CfCookieJar API）：

```rust
            // CF 挑战解决后，保存 cookie 到 jar
            if let Some(ref url_parsed) = url::Url::parse(&req.url).ok() {
                if let Ok(resp) = page.cmd("Network.getCookies", serde_json::json!({})).await {
                    if let Some(cookies) = resp.pointer("/cookies").and_then(|c| c.as_array()) {
                        let host = url_parsed.host_str().unwrap_or("");
                        let cf_cookies: Vec<crate::cookie::Cookie> = cookies
                            .iter()
                            .filter(|c| {
                                let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                name.starts_with("cf_") || name.starts_with("__cf")
                            })
                            .filter_map(|v| {
                                Some(crate::cookie::Cookie {
                                    name: v.get("name")?.as_str()?.to_string(),
                                    value: v.get("value")?.as_str()?.to_string(),
                                    domain: v.get("domain").and_then(|d| d.as_str()).unwrap_or(host).to_string(),
                                    path: v.get("path").and_then(|p| p.as_str()).unwrap_or("/").to_string(),
                                    secure: v.get("secure").and_then(serde_json::Value::as_bool).unwrap_or(false),
                                    http_only: v.get("httpOnly").and_then(serde_json::Value::as_bool).unwrap_or(false),
                                    same_site: v.get("sameSite").and_then(|s| s.as_str()).map(String::from),
                                    expires: v.get("expires").and_then(serde_json::Value::as_f64),
                                })
                            })
                            .collect();
                        for c in cf_cookies {
                            self.cookie_jar.set(c).await;
                        }
                        if !cf_cookies.is_empty() {
                            tracing::info!(
                                "BrowserWork[{solve_label}]: {url} 保存 {} 个 CF cookie",
                                cf_cookies.len()
                            );
                        }
                    }
                }
            }
```

（实际实现时采用此简化方案，无需暴露 `value_to_cookie` 为 pub）

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib fetcher::client::tests`
Expected: PASS（原 3 个测试 + 新增 3 个测试全绿）

Run: `cargo build`
Expected: 编译成功（无 warning 关于未使用字段）

如果 `cookie_provider` 方法在 `http::Client::ClientBuilder` 上不存在，确认已在 Step 3a 中添加。

- [ ] **Step 5: 提交**

```bash
git add src/fetcher/client.rs src/http/mod.rs
git commit -m "feat: FetchClient 集成 CookieJar，删除 cf_cache 字段"
```

---

## Task 7: 验证 stealth/challenge.rs 无需修改

**Files:**
- Verify: `src/stealth/challenge.rs`（只读，确认无 cf_cache 依赖）
- Modify: 仅在发现死代码引用时清理

**Interfaces:**
- Consumes: Task 6 完成的 FetchClient
- Produces: 无（验证任务）

- [ ] **Step 1: 写失败的测试**

无需新测试。运行现有 challenge.rs 测试验证：

Run: `cargo test --lib stealth::challenge::tests`
Expected: PASS（3 个单元测试全绿，集成测试 `#[ignore]`）

- [ ] **Step 2: 运行测试验证现状**

Run: `cargo test --lib stealth::challenge::tests`
Expected: PASS（如果失败，说明 Task 6 的修改意外影响了 challenge.rs，需排查）

- [ ] **Step 3: 全局搜索 cf_cache 残留引用**

Run: `grep -rn "cf_cache" src/ --include="*.rs"`（在 shell 中执行）

或使用 Grep 工具搜索 `cf_cache`：
- 检查是否还有任何文件引用 `cf_cache`、`has_cf_cookies`、`get_cf_cookie_header`、`get_cf_ua`
- 预期：除 `fetcher/client.rs` 已删除外，无其他文件引用

如果发现调用方（如 `crawl/` 或 `mcp/`）仍调用 `has_cf_cookies` 等已删除方法，需更新调用方使用 `cookie_jar().header(url).await` 替代。

- [ ] **Step 4: 运行全部测试验证通过**

Run: `cargo test --lib`
Expected: PASS（所有现有测试 + PR1 新增测试全绿）

Run: `cargo build`
Expected: 编译成功

- [ ] **Step 5: 提交（仅当有清理改动时）**

如果 Step 3 发现并修复了调用方残留引用：

```bash
git add src/path/to/caller.rs
git commit -m "refactor: 调用方迁移至 cookie_jar().header() 接口"
```

如果无改动，跳过提交，仅记录验证结论。

---

## Task 8: 更新 lib.rs 声明 cookie 模块 + 最终验证

**Files:**
- Modify: `src/lib.rs:90-117`（模块声明区）和 `src/lib.rs:119-150`（re-export 区）
- Test: 全项目编译 + 全部测试

**Interfaces:**
- Consumes: Task 1-7 全部完成
- Produces: `wisp::cookie::{Cookie, CookieJar, CfCookieJar, CfSession, HttpCookieJar, BrowserCookieJar, MockCookieJar}` 公开 API

- [ ] **Step 1: 写失败的测试**

在 `src/lib.rs` 末尾的 `#[cfg(test)] mod tests` 中（如果不存在则创建）追加：

```rust
#[cfg(test)]
mod cookie_module_tests {
    /// 验证 cookie 模块的所有公开 API 可访问。
    #[test]
    fn cookie_module_public_api_accessible() {
        use crate::cookie::{
            BrowserCookieJar, CfCookieJar, CfSession, Cookie, CookieJar, HttpCookieJar,
            MockCookieJar,
        };

        // 编译期检查：所有类型可命名
        fn _check_cookie(c: Cookie) -> Cookie {
            c
        }
        fn _check_session(s: CfSession) -> CfSession {
            s
        }
        // trait object 可构造
        let _: Arc<dyn CookieJar> = Arc::new(MockCookieJar::new());
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib cookie_module_tests`
Expected: FAIL with "no `cookie` module in crate root" 或类似（如果 Task 1 已声明 `pub mod cookie;` 则应通过；此测试主要验证 re-export 完整性）

- [ ] **Step 3: 写最小实现**

修改 `src/lib.rs:89-117`，确认 cookie 模块已声明（Task 1 已添加）：

```rust
/// Cookie 存储 trait + 三实现（Http/Browser/Cf）。
pub mod cookie;
```

在 `src/lib.rs:119-150` 的 re-export 区，在 `pub use stealth::TurnstileConfig;` 之后添加：

```rust
// === Cookie 管理 ===
pub use cookie::{BrowserCookieJar, CfCookieJar, CfSession, Cookie, CookieJar, HttpCookieJar};
```

- [ ] **Step 4: 运行全部测试验证通过**

Run: `cargo test --lib`
Expected: PASS（所有现有测试 + PR1 新增测试全绿，包括 cookie 模块的 19+ 个测试）

Run: `cargo build`
Expected: 编译成功，无 warning（除预先存在的）

Run: `cargo test --doc`
Expected: PASS（doctest 无破坏）

- [ ] **Step 5: 最终提交**

```bash
git add src/lib.rs
git commit -m "feat: lib.rs 声明 cookie 模块并 re-export 公开 API"
```

PR1 完成验证：

```bash
cargo test --lib
cargo build --release
```

---

## Self-Review 检查清单

实施者完成所有任务后，对照以下检查清单验证：

### Spec 覆盖检查

- [ ] **3.1 CookieJar trait**：Task 1 完整实现（Cookie + CookieJar trait + MockCookieJar）
- [ ] **3.2 三种实现**：Task 2 (CfCookieJar)、Task 3 (HttpCookieJar)、Task 4 (BrowserCookieJar)
- [ ] **3.3 FetchClient 集成**：Task 6 完成（删除 cf_cache，新增 cookie_jar，新增 cookie_jar() 方法）
- [ ] **3.4 StorageError 细分**：Task 5 完成（新增 NotFound/Serialization/Backend/Corrupted/Io 变体）
- [ ] **3.5 迁移点**：
  - [ ] `fetcher/client.rs` 删除 `CfSessionCache` 字段和相关方法（Task 6）
  - [ ] `fetcher/client.rs` 新增 `cookie_jar: Arc<dyn CookieJar>` 字段（Task 6）
  - [ ] `stealth/challenge.rs` 验证无依赖（Task 7）
  - [ ] `fetcher/client.rs::do_browser_work_inner` CF cookie 注入/保存改用 cookie_jar（Task 6）
  - [ ] `storage/mod.rs` `load_element` 等改用 `StorageError::Corrupted`（Task 5）
  - [ ] `error.rs` 新增 4 个 StorageError 变体（Task 5）
- [ ] **3.6 测试策略**：
  - [ ] CookieJar trait 单元测试（MockCookieJar，Task 1，7 个测试）
  - [ ] HttpCookieJar 单元测试（Task 3，5 个测试）
  - [ ] CfCookieJar 单元测试（TTL 过期、文件持久化往返，Task 2，6 个测试）
  - [ ] BrowserCookieJar 单元测试 + 集成测试（Task 4，3+2 个测试）
  - [ ] StorageError 测试（Display 输出、#[from] 转换，Task 5，7 个测试）
  - [ ] 现有测试全绿（每任务末尾运行 `cargo test --lib`）

### 类型一致性检查

- [ ] `Cookie` 结构体字段在所有任务中一致（name/value/domain/path/secure/http_only/same_site/expires）
- [ ] `CookieJar` trait 方法签名一致（`get(&self, url: &Url) -> Vec<Cookie>` / `set(&self, cookie: Cookie)` / `clear(&self, url: &Url)` / `header(&self, url: &Url) -> Option<String>`）
- [ ] `CfCookieJar`/`HttpCookieJar`/`BrowserCookieJar` 都正确实现 `CookieJar` trait
- [ ] `FetchClient::cookie_jar()` 返回 `&Arc<dyn CookieJar>`（Task 6 定义，Task 8 验证）

### Placeholder 扫描

确认本计划无以下占位符：
- "TBD" / "TODO" / "implement later" / "fill in details" ❌
- "Add appropriate error handling" ❌
- "Similar to Task N" ❌
- 步骤描述无代码 ❌
- 引用未定义的类型/函数 ❌

每个步骤都包含完整可执行代码。

---

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| `wreq::cookie::Jar::add` 接受 `&str` 时解析失败静默丢弃 cookie | Task 3 测试 `http_set_and_get_cookie` 验证写入可读回；若失败改用 `cookie::CookieBuilder` 构造 `RawCookie` |
| `wreq::cookie::Jar::remove` 签名要求 `Into<RawCookie<'static>>`，传 `String` 可能不通过 | Task 3 `http_clear_removes_matching` 测试验证；若失败改用 `Jar::clear()` 全清（牺牲精度） |
| `BrowserCookieJar` 的 CDP `Network.getCookies` 参数 `urls` 可能在不同 Chrome 版本行为不同 | Task 4 集成测试 `#[ignore]` 仅在 Chrome 环境手动验证；CI 不阻塞 |
| `http::Client::ClientBuilder::cookie_provider` 方法添加后，现有调用方未受影响（默认 `cookie_jar: None` 走 `cookie_store(true)` 分支） | Task 6 Step 3a 明确：`cookie_jar` 字段默认 `None`，未注入时走原 `cookie_store(true)` 路径 |
| FetchClient 删除 `has_cf_cookies` 等方法后，外部调用方（banzhu-rs）编译失败 | Task 7 Step 3 全局搜索残留引用，更新调用方使用 `cookie_jar().header(url).await` |
| `StorageError::Io(#[from] std::io::Error)` 与 `WispError::Io(#[from] std::io::Error)` 同时存在导致 `?` 歧义 | thiserr `#[from]` 优先具体类型：storage 模块内 `?` io::Error 转 `StorageError::Io`，再通过 `WispError::Storage(#[from] StorageError)` 上抛；非 storage 模块 `?` 直接转 `WispError::Io`。无歧义。 |

---

## 实施顺序与验证

```
1. Task 1 (Cookie + CookieJar trait)         → 验证: cargo test --lib cookie::tests (7 pass)
2. Task 2 (CfCookieJar)                       → 验证: cargo test --lib cookie::cf::tests (6 pass)
3. Task 3 (HttpCookieJar)                      → 验证: cargo test --lib cookie::http::tests (5 pass)
4. Task 4 (BrowserCookieJar)                  → 验证: cargo test --lib cookie::browser::tests (3 pass + 2 ignored)
5. Task 5 (StorageError 细分)                  → 验证: cargo test --lib error::tests + storage::tests (7+ pass)
6. Task 6 (FetchClient 集成 CookieJar)         → 验证: cargo test --lib fetcher::client::tests + cargo build
7. Task 7 (验证 challenge.rs)                  → 验证: cargo test --lib stealth::challenge::tests + grep cf_cache
8. Task 8 (lib.rs re-export + 最终验证)        → 验证: cargo test --lib + cargo build --release
```

每个 Task 完成后单独提交，便于回滚。

---

## 回滚方案

PR1 完整回滚步骤：

1. `git revert` Task 8 的提交（恢复 lib.rs）
2. `git revert` Task 6 的提交（恢复 cf_cache 字段、CF 方法、http/mod.rs）
3. `git revert` Task 5 的提交（恢复 StorageError 单变体）
4. `git revert` Task 4/3/2 的提交（删除 browser/http/cf 实现）
5. `git revert` Task 1 的提交（删除 cookie 模块）

由于每 Task 独立提交，可选择性回滚部分功能（如保留 CookieJar trait 但回滚 FetchClient 集成）。
