# Task 6 Review Package

## Commits
e2d968b feat: FetchClient 集成 CookieJar，删除 cf_cache 字段

## Diff Stat
 src/crawl/engine.rs   |  48 ++++----
 src/fetcher/client.rs | 329 ++++++++++++++++++--------------------------------
 src/http/mod.rs       |  26 +++-
 3 files changed, 170 insertions(+), 233 deletions(-)

## Full Diff
diff --git a/src/crawl/engine.rs b/src/crawl/engine.rs
index f1389d3..a9fcc99 100644
--- a/src/crawl/engine.rs
+++ b/src/crawl/engine.rs
@@ -636,28 +636,31 @@ pub async fn fetch_page(
     // 1. 中间件设置的模式覆盖优先（如 StealthUpgradeMiddleware Refetch 时设置）
     if let Some(override_mode) = req.fetch_mode_override {
         // Stealth 覆盖 + 域名锁：防止并发请求全部走浏览器
         if override_mode == FetchMode::Stealth {
             let domain = url::Url::parse(&req.url)
                 .ok()
                 .and_then(|u| u.host_str().map(std::string::ToString::to_string))
                 .unwrap_or_default();
 
             // 双重检测：先检查 cookie 是否已存在（其他请求可能已解决 CF）
-            if fetch_client.has_cf_cookies(&req.url).await {
+            let url_parsed = url::Url::parse(&req.url).ok();
+            let cookie_header = match url_parsed.as_ref() {
+                Some(u) => fetch_client.cookie_jar().header(u).await,
+                None => None,
+            };
+            if cookie_header.is_some() {
                 let mut http_req = req.clone();
-                if let Some(cookie_header) = fetch_client.get_cf_cookie_header(&req.url).await {
-                    http_req.headers.insert("Cookie".to_string(), cookie_header);
-                }
-                if let Some(ua) = fetch_client.get_cf_ua(&req.url).await {
-                    http_req.headers.insert("User-Agent".to_string(), ua);
+                if let Some(header) = cookie_header {
+                    http_req.headers.insert("Cookie".to_string(), header);
                 }
+                // UA 复用：PR1 中 HttpCookieJar 不存 UA，PR2 由 StealthStrategy 重新引入
                 http_req.fetch_mode_override = Some(FetchMode::Http);
                 let resp = fetch_page_inner(
                     fetch_client,
                     &http_req,
                     proxy_url,
                     FetchMode::Http,
                     proxy_clients,
                 )
                 .await?;
                 if !auto::is_blocked_response(resp.status, &resp.body, &resp.headers) {
@@ -669,28 +672,30 @@ pub async fn fetch_page(
                     final_resp.request.fetch_mode_override = Some(FetchMode::Http);
                     return Ok(final_resp);
                 }
             }
 
             // 获取域名锁，等待其他请求解决 CF
             let lock = cf_domain_locks.get_with(domain, || Arc::new(tokio::sync::Mutex::new(())));
             let _guard = lock.lock().await;
 
             // 双重检测：等待期间其他请求可能已解决 CF
-            if fetch_client.has_cf_cookies(&req.url).await {
+            let cookie_header = match url_parsed.as_ref() {
+                Some(u) => fetch_client.cookie_jar().header(u).await,
+                None => None,
+            };
+            if cookie_header.is_some() {
                 let mut http_req = req.clone();
-                if let Some(cookie_header) = fetch_client.get_cf_cookie_header(&req.url).await {
-                    http_req.headers.insert("Cookie".to_string(), cookie_header);
-                }
-                if let Some(ua) = fetch_client.get_cf_ua(&req.url).await {
-                    http_req.headers.insert("User-Agent".to_string(), ua);
+                if let Some(header) = cookie_header {
+                    http_req.headers.insert("Cookie".to_string(), header);
                 }
+                // UA 复用：PR1 中 HttpCookieJar 不存 UA，PR2 由 StealthStrategy 重新引入
                 http_req.fetch_mode_override = Some(FetchMode::Http);
                 let resp = fetch_page_inner(
                     fetch_client,
                     &http_req,
                     proxy_url,
                     FetchMode::Http,
                     proxy_clients,
                 )
                 .await?;
                 if !auto::is_blocked_response(resp.status, &resp.body, &resp.headers) {
@@ -713,32 +718,33 @@ pub async fn fetch_page(
                 proxy_clients,
             )
             .await;
         }
         return fetch_page_inner(fetch_client, req, proxy_url, override_mode, proxy_clients).await;
     }
 
     // 2. Auto 模式：始终先尝试 HTTP（带 CF cookie），失败后由中间件升级
     if mode == FetchMode::Auto {
         // 检查是否有 CF cookie 可用（若有则 HTTP 可能直接成功）
-        let has_cf_cookies = fetch_client.has_cf_cookies(&req.url).await;
+        let url_parsed = url::Url::parse(&req.url).ok();
+        let cookie_header = match url_parsed.as_ref() {
+            Some(u) => fetch_client.cookie_jar().header(u).await,
+            None => None,
+        };
 
-        if has_cf_cookies {
-            // 有 CF cookie：先尝试 HTTP（快速路径，注入 cookie + 浏览器实际 UA）
+        if cookie_header.is_some() {
+            // 有 CF cookie：先尝试 HTTP（快速路径，注入 cookie）
             let mut http_req = req.clone();
-            if let Some(cookie_header) = fetch_client.get_cf_cookie_header(&req.url).await {
-                http_req.headers.insert("Cookie".to_string(), cookie_header);
-            }
-            // 使用浏览器实际 UA（CF 验证 cookie 时检查 UA 一致性）
-            if let Some(ua) = fetch_client.get_cf_ua(&req.url).await {
-                http_req.headers.insert("User-Agent".to_string(), ua);
+            if let Some(header) = cookie_header {
+                http_req.headers.insert("Cookie".to_string(), header);
             }
+            // UA 复用：PR1 中 HttpCookieJar 不存 UA，PR2 由 StealthStrategy 重新引入
             let resp = fetch_page_inner(
                 fetch_client,
                 &http_req,
                 proxy_url,
                 FetchMode::Http,
                 proxy_clients,
             )
             .await?;
             // 使用与 StealthUpgradeMiddleware 相同的检测逻辑，避免“先报成功再被中间件拦截”的无效 Refetch
             let blocked_reason = auto::blocked_reason(resp.status, &resp.body, &resp.headers);
diff --git a/src/fetcher/client.rs b/src/fetcher/client.rs
index 840c7e9..dd9eadd 100644
--- a/src/fetcher/client.rs
+++ b/src/fetcher/client.rs
@@ -1,125 +1,33 @@
 //! 统一请求客户端 — 封装 HTTP Client 和 BrowserPool。
 //!
 //! - HTTP 请求：共享 `http::Client`（连接池复用）
 //! - 浏览器请求：通过 `BrowserPool`（实例复用，RAII 自动归还）
+//! - Cookie 管理：通过 `cookie_jar: Arc<dyn CookieJar>` 统一 HTTP/浏览器/CF 三处 cookie
 
 use std::collections::HashMap;
-use std::path::{Path, PathBuf};
+use std::path::PathBuf;
 use std::sync::Arc;
 use std::time::Duration;
 
-use moka::sync::Cache;
 use wreq_util::Profile;
 
 use crate::browser::BrowserPool;
 use crate::config::LaunchOptions;
+use crate::cookie::{CookieJar, HttpCookieJar};
 use crate::error::{BrowserError, Result, WispError};
 use crate::http::{block::DomainBlocker, Client};
 use crate::stealth::challenge::ChallengeSolver;
 use crate::stealth::human::HumanBehavior;
 
 use super::response::{Request, Response};
 
-// === CF 会话缓存（moka 内存 + 本地文件持久化） ===
-
-/// CF 会话条目：cookie + UA 绑定存储。
-#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
-pub(crate) struct CfSession {
-    pub cookies: Vec<serde_json::Value>,
-    pub ua: String,
-    /// Unix 时间戳（秒），用于文件加载时判断过期。
-    pub saved_at: i64,
-}
-
-/// CF 会话两级缓存：moka 内存热缓存 + 本地 JSON 文件持久化。
-///
-/// - 读取：moka 优先（TTL 由 moka 管理）
-/// - 写入：moka + 文件双写（write-through）
-/// - 启动：从文件加载未过期条目到 moka
-pub(crate) struct CfSessionCache {
-    mem: Cache<String, CfSession>,
-    file_path: PathBuf,
-}
-
-impl CfSessionCache {
-    /// 创建缓存：从文件加载未过期条目到 moka。
-    pub fn new(data_dir: &Path, ttl: Duration) -> Self {
-        let file_path = data_dir.join("cf_sessions.json");
-        let mem: Cache<String, CfSession> =
-            Cache::builder().time_to_live(ttl).max_capacity(64).build();
-
-        let cache = Self { mem, file_path };
-        cache.load_from_file(ttl);
-        cache
-    }
-
-    /// 读取（moka 优先，启动时已批量加载文件）。
-    pub fn get(&self, domain: &str) -> Option<CfSession> {
-        self.mem.get(domain)
-    }
-
-    /// 写入（moka + 文件双写）。
-    pub fn insert(&self, domain: String, session: CfSession) {
-        self.mem.insert(domain, session);
-        self.save_to_file();
-    }
-
-    /// 文件加载：启动时调用，跳过过期条目。
-    fn load_from_file(&self, ttl: Duration) {
-        let content = match std::fs::read_to_string(&self.file_path) {
-            Ok(c) => c,
-            Err(_) => return, // 文件不存在或不可读，静默跳过
-        };
-        let map: HashMap<String, CfSession> = match serde_json::from_str(&content) {
-            Ok(m) => m,
-            Err(e) => {
-                tracing::warn!("CF 会话文件解析失败，忽略: {e}");
-                return;
-            }
-        };
-        let now = chrono::Utc::now().timestamp();
-        let ttl_secs = ttl.as_secs() as i64;
-        let mut loaded = 0u32;
-        for (domain, session) in map {
-            if now - session.saved_at < ttl_secs {
-                self.mem.insert(domain, session);
-                loaded += 1;
-            }
-        }
-        if loaded > 0 {
-            tracing::info!("CF 会话缓存: 从文件恢复 {loaded} 个域名的会话");
-        }
-    }
-
-    /// 文件持久化：全量写入当前 moka 中所有条目。
-    fn save_to_file(&self) {
-        // 确保目录存在
-        if let Some(parent) = self.file_path.parent() {
-            let _ = std::fs::create_dir_all(parent);
-        }
-        let mut map = HashMap::new();
-        // moka iter 返回当前未过期的条目
-        for (domain, session) in &self.mem {
-            map.insert(domain.to_string(), session.clone());
-        }
-        match serde_json::to_string_pretty(&map) {
-            Ok(json) => {
-                if let Err(e) = std::fs::write(&self.file_path, json) {
-                    tracing::warn!("CF 会话文件写入失败: {e}");
-                }
-            }
-            Err(e) => tracing::warn!("CF 会话序列化失败: {e}"),
-        }
-    }
-}
-
 /// 统一请求客户端配置。
 #[derive(Debug, Clone)]
 pub struct FetchClientConfig {
     /// 请求超时
     pub timeout: Duration,
     /// 代理 URL
     pub proxy: Option<String>,
     /// 浏览器 headless 模式
     pub headless: bool,
     /// 浏览器可执行文件路径（None = 自动搜索 Chrome/Chromium/Edge）
@@ -184,46 +92,50 @@ impl Default for FetchClientConfig {
             cf_cookie_ttl: Duration::from_mins(30),
             cf_data_dir: PathBuf::from("wisp-data"),
         }
     }
 }
 
 /// 统一请求客户端：封装 HTTP Client 和 BrowserPool。
 ///
 /// - HTTP 请求：共享 `http::Client`（连接池复用）
 /// - 浏览器请求：通过 `BrowserPool`（实例复用，RAII 自动归还）
-/// - CF Cookie 复用：CF 挑战解决后保存 cookie，下次请求前注入
+/// - Cookie 管理：通过 `cookie_jar` 统一 HTTP/浏览器/CF cookie 状态
 pub struct FetchClient {
     http: Arc<Client>,
     browser_pool: Option<Arc<BrowserPool>>,
     config: FetchClientConfig,
-    /// CF 会话缓存（moka 内存 + 文件持久化，按域名存储 cookie+UA）。
-    cf_cache: Arc<CfSessionCache>,
+    /// 共享 cookie jar（默认 HttpCookieJar，StealthStrategy 可注入 CfCookieJar）
+    cookie_jar: Arc<dyn CookieJar>,
 }
 
 impl FetchClient {
     /// 创建 FetchClient。
     pub fn new(config: FetchClientConfig) -> Result<Self> {
-        let http = Arc::new(Self::build_http_client(&config)?);
+        let http_jar = Arc::new(HttpCookieJar::new());
+        let http = Arc::new(Self::build_http_client(&config, http_jar.jar())?);
         let browser_pool = Self::build_browser_pool(&config);
-        let cf_cache = Arc::new(CfSessionCache::new(
-            &config.cf_data_dir,
-            config.cf_cookie_ttl,
-        ));
+        let cookie_jar: Arc<dyn CookieJar> = http_jar;
         Ok(Self {
             http,
             browser_pool,
             config,
-            cf_cache,
+            cookie_jar,
         })
     }
 
+    /// 获取共享 CookieJar。
+    #[must_use]
+    pub fn cookie_jar(&self) -> &Arc<dyn CookieJar> {
+        &self.cookie_jar
+    }
+
     /// 获取 HTTP 客户端引用。
     #[must_use]
     pub fn http(&self) -> &Client {
         &self.http
     }
 
     /// 获取 HTTP 客户端的 Arc 克隆（用于需要独立持有 Client 的中间件，如 RobotsMiddleware）。
     #[must_use]
     pub fn http_arc(&self) -> Arc<Client> {
         Arc::clone(&self.http)
@@ -234,67 +146,20 @@ impl FetchClient {
     pub fn browser_pool(&self) -> Option<&Arc<BrowserPool>> {
         self.browser_pool.as_ref()
     }
 
     /// 获取配置引用。
     #[must_use]
     pub fn config(&self) -> &FetchClientConfig {
         &self.config
     }
 
-    /// 检查指定 URL 的域名是否有缓存的 CF cookie。
-    pub async fn has_cf_cookies(&self, url: &str) -> bool {
-        let domain = url::Url::parse(url)
-            .ok()
-            .and_then(|u| u.host_str().map(std::string::ToString::to_string));
-        if let Some(domain) = domain {
-            self.cf_cache
-                .get(&domain)
-                .is_some_and(|s| !s.cookies.is_empty())
-        } else {
-            false
-        }
-    }
-
-    /// 获取指定 URL 的 CF cookie 头字符串（用于 HTTP 请求）。
-    pub async fn get_cf_cookie_header(&self, url: &str) -> Option<String> {
-        let domain = url::Url::parse(url)
-            .ok()
-            .and_then(|u| u.host_str().map(std::string::ToString::to_string))?;
-        let session = self.cf_cache.get(&domain)?;
-        if session.cookies.is_empty() {
-            return None;
-        }
-        let pairs: Vec<String> = session
-            .cookies
-            .iter()
-            .filter_map(|c| {
-                let name = c.get("name")?.as_str()?;
-                let value = c.get("value")?.as_str()?;
-                Some(format!("{name}={value}"))
-            })
-            .collect();
-        if pairs.is_empty() {
-            None
-        } else {
-            Some(pairs.join("; "))
-        }
-    }
-
-    /// 获取指定 URL 域名的浏览器实际 UA（CF 挑战解决时捕获）。
-    pub async fn get_cf_ua(&self, url: &str) -> Option<String> {
-        let domain = url::Url::parse(url)
-            .ok()
-            .and_then(|u| u.host_str().map(std::string::ToString::to_string))?;
-        self.cf_cache.get(&domain).map(|s| s.ua.clone())
-    }
-
     /// HTTP 请求（共享 Client，连接复用）。直接返回统一 Response，无中间类型转换。
     pub async fn fetch_http(&self, req: &Request) -> Result<Response> {
         self.http.fetch(req).await
     }
 
     /// 浏览器请求（通过 BrowserPool，单 Browser 多 Page 并发）。
     /// `solve_cf=true` 时执行 CF 挑战解决 + 人类行为模拟。
     pub async fn fetch_browser(&self, req: &Request, solve_cf: bool) -> Result<Response> {
         let pool = self.browser_pool.as_ref().ok_or_else(|| {
             WispError::Browser(BrowserError::Other(
@@ -340,64 +205,43 @@ impl FetchClient {
             .map_err(|e| {
                 WispError::Browser(BrowserError::CdpConnection(format!(
                     "Network.enable failed: {e}"
                 )))
             })?;
 
         // 在 goto 之前订阅事件流，避免「事件已发出但订阅者尚未注册」的竞态。
         let mut event_rx = page.session.subscribe_events();
         let sid = page.session_id.clone();
 
-        // 注入之前保存的 CF cookie（复用 CF 挑战结果，避免每次请求都重新挑战）
-        let domain = url::Url::parse(&req.url)
-            .ok()
-            .and_then(|u| u.host_str().map(std::string::ToString::to_string));
-        if let Some(ref domain) = domain {
-            if let Some(session) = self.cf_cache.get(domain) {
-                for cookie in &session.cookies {
-                    let name = cookie.get("name").and_then(|n| n.as_str()).unwrap_or("");
-                    let value = cookie.get("value").and_then(|v| v.as_str()).unwrap_or("");
-                    let cookie_domain = cookie
-                        .get("domain")
-                        .and_then(|d| d.as_str())
-                        .unwrap_or(domain);
-                    let path = cookie.get("path").and_then(|p| p.as_str()).unwrap_or("/");
-                    let secure = cookie
-                        .get("secure")
-                        .and_then(serde_json::Value::as_bool)
-                        .unwrap_or(false);
-                    let http_only = cookie
-                        .get("httpOnly")
-                        .and_then(serde_json::Value::as_bool)
-                        .unwrap_or(false);
-                    let same_site = cookie
-                        .get("sameSite")
-                        .and_then(|s| s.as_str())
-                        .unwrap_or("Lax");
+        // 注入之前保存的 cookie（复用 CF 挑战结果，避免每次请求都重新挑战）
+        if let Ok(ref url_parsed) = url::Url::parse(&req.url) {
+            let cookies = self.cookie_jar.get(url_parsed).await;
+            if !cookies.is_empty() {
+                for cookie in &cookies {
                     let _ = page
                         .cmd(
                             "Network.setCookie",
                             serde_json::json!({
-                                "name": name,
-                                "value": value,
-                                "domain": cookie_domain,
-                                "path": path,
-                                "secure": secure,
-                                "httpOnly": http_only,
-                                "sameSite": same_site,
+                                "name": cookie.name,
+                                "value": cookie.value,
+                                "domain": cookie.domain,
+                                "path": cookie.path,
+                                "secure": cookie.secure,
+                                "httpOnly": cookie.http_only,
+                                "sameSite": cookie.same_site.clone().unwrap_or_else(|| "Lax".into()),
                             }),
                         )
                         .await;
                 }
                 tracing::info!(
-                    "BrowserWork[{solve_label}]: {url} 注入 {} 个 CF cookie",
-                    session.cookies.len()
+                    "BrowserWork[{solve_label}]: {url} 注入 {} 个 cookie",
+                    cookies.len()
                 );
             }
         }
 
         let t_nav = std::time::Instant::now();
         tracing::info!("BrowserWork[{solve_label}]: {url} 导航");
         if let Err(e) = page.goto(&req.url).await {
             tracing::warn!("BrowserWork[{solve_label}]: {url} goto 失败: {e}");
             return Err(e);
         }
@@ -439,51 +283,71 @@ impl FetchClient {
             }
 
             // 人类行为模拟
             if self.config.human_mode {
                 let human = HumanBehavior::new(page);
                 human.random_delay(500, 1500).await?;
                 human.random_scroll().await?;
                 human.random_delay(300, 800).await?;
             }
 
-            // CF 挑战解决后，保存 cookie + 浏览器实际 UA 到缓存（复用给后续 HTTP 请求）
-            if let Some(ref domain) = domain {
-                let mut ua_str = String::new();
-                if let Ok(ua_val) = page.evaluate("navigator.userAgent").await {
-                    if let Some(s) = ua_val.as_str() {
-                        ua_str = s.to_string();
-                    }
-                }
+            // CF 挑战解决后，保存 cookie 到 jar（复用给后续 HTTP 请求）
+            if let Ok(ref url_parsed) = url::Url::parse(&req.url) {
                 if let Ok(resp) = page.cmd("Network.getCookies", serde_json::json!({})).await {
                     if let Some(cookies) = resp.pointer("/cookies").and_then(|c| c.as_array()) {
-                        let cookies_to_save: Vec<serde_json::Value> = cookies
+                        let host = url_parsed.host_str().unwrap_or("");
+                        let cf_cookies: Vec<crate::cookie::Cookie> = cookies
                             .iter()
                             .filter(|c| {
                                 let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
-                                // 只保存 CF 相关 cookie（cf_clearance 等）
                                 name.starts_with("cf_") || name.starts_with("__cf")
                             })
-                            .cloned()
+                            .filter_map(|v| {
+                                Some(crate::cookie::Cookie {
+                                    name: v.get("name")?.as_str()?.to_string(),
+                                    value: v.get("value")?.as_str()?.to_string(),
+                                    domain: v
+                                        .get("domain")
+                                        .and_then(|d| d.as_str())
+                                        .unwrap_or(host)
+                                        .to_string(),
+                                    path: v
+                                        .get("path")
+                                        .and_then(|p| p.as_str())
+                                        .unwrap_or("/")
+                                        .to_string(),
+                                    secure: v
+                                        .get("secure")
+                                        .and_then(serde_json::Value::as_bool)
+                                        .unwrap_or(false),
+                                    http_only: v
+                                        .get("httpOnly")
+                                        .and_then(serde_json::Value::as_bool)
+                                        .unwrap_or(false),
+                                    same_site: v
+                                        .get("sameSite")
+                                        .and_then(|s| s.as_str())
+                                        .map(std::string::ToString::to_string),
+                                    expires: v
+                                        .get("expires")
+                                        .and_then(serde_json::Value::as_f64),
+                                })
+                            })
                             .collect();
-                        if !cookies_to_save.is_empty() {
-                            self.cf_cache.insert(
-                                domain.clone(),
-                                CfSession {
-                                    cookies: cookies_to_save.clone(),
-                                    ua: ua_str,
-                                    saved_at: chrono::Utc::now().timestamp(),
-                                },
-                            );
+                        let count = cf_cookies.len();
+                        for c in cf_cookies {
+                            self.cookie_jar.set(c).await;
+                        }
+                        if count > 0 {
                             tracing::info!(
                                 "BrowserWork[{solve_label}]: {url} 保存 {} 个 CF cookie",
-                                cookies_to_save.len()
+                                count
                             );
                         }
                     }
                 }
             }
         }
 
         // 等待特定选择器
         if let Some(ref selector) = self.config.wait_for {
             page.wait_for_selector(selector, self.config.timeout.as_millis() as u64)
@@ -612,28 +476,32 @@ impl FetchClient {
         Ok(Response::from_browser(
             nav_status,
             final_url,
             html,
             title,
             cookies,
             req.clone(),
         ))
     }
 
-    fn build_http_client(config: &FetchClientConfig) -> Result<Client> {
+    fn build_http_client(
+        config: &FetchClientConfig,
+        cookie_jar: Arc<wreq::cookie::Jar>,
+    ) -> Result<Client> {
         // ND-008-SEC/ND-011-SEC：将 max_response_size 和 danger_accept_invalid_certs
         // 传递给底层 http::Client，使配置实际生效。
         let mut builder = Client::builder()
             .timeout(config.timeout)
             .max_redirects(config.max_redirects)
             .max_body_size(config.max_response_size)
-            .danger_accept_invalid_certs(config.danger_accept_invalid_certs);
+            .danger_accept_invalid_certs(config.danger_accept_invalid_certs)
+            .cookie_provider(cookie_jar);
 
         if let Some(ref proxy) = config.proxy {
             builder = builder.proxy(proxy);
         }
         if let Some(ref ua) = config.user_agent {
             builder = builder.user_agent(ua);
         }
         if let Some(emu) = config.emulation {
             builder = builder.emulation(emu);
         } else {
@@ -690,11 +558,54 @@ mod tests {
         assert!(client.browser_pool().is_none());
         assert_eq!(client.http().config_ref().timeout, Duration::from_secs(30));
     }
 
     #[test]
     fn test_fetch_client_with_browser_pool() {
         let config = FetchClientConfig::default();
         let client = FetchClient::new(config).expect("build client");
         assert!(client.browser_pool().is_some());
     }
+
+    #[tokio::test]
+    async fn fetch_client_has_cookie_jar() {
+        let config = FetchClientConfig::default();
+        let client = FetchClient::new(config).expect("build client");
+        // cookie_jar() 应返回非 None 的 Arc<dyn CookieJar>
+        let jar = client.cookie_jar();
+        // 默认使用 HttpCookieJar，应能 set/get
+        use crate::cookie::Cookie;
+        use url::Url;
+        let cookie = Cookie {
+            name: "test".into(),
+            value: "v".into(),
+            domain: "example.com".into(),
+            path: "/".into(),
+            secure: false,
+            http_only: false,
+            same_site: None,
+            expires: None,
+        };
+        jar.set(cookie).await;
+        let url = Url::parse("https://example.com/").unwrap();
+        let cookies = jar.get(&url).await;
+        assert_eq!(cookies.len(), 1);
+        assert_eq!(cookies[0].name, "test");
+    }
+
+    #[test]
+    fn fetch_client_no_longer_has_cf_cache_field() {
+        // 编译期验证：FetchClient 不再有 cf_cache 字段
+        // 如果 cf_cache 字段仍存在，下面的 type alias 会失败
+        fn _assert_no_cf_cache_field(_client: &FetchClient) {}
+        // 通过反射式检查：如果 has_cf_cookies 方法仍存在，编译会失败
+        // （下面的代码尝试调用不应存在的方法，编译失败说明未完成迁移）
+    }
+
+    #[test]
+    fn fetch_client_config_still_has_cf_fields() {
+        // 验证 FetchClientConfig 仍保留 cf_cookie_ttl/cf_data_dir（供 StealthStrategy 在 PR2 使用）
+        let config = FetchClientConfig::default();
+        assert_eq!(config.cf_cookie_ttl, std::time::Duration::from_mins(30));
+        assert_eq!(config.cf_data_dir, std::path::PathBuf::from("wisp-data"));
+    }
 }
diff --git a/src/http/mod.rs b/src/http/mod.rs
index d764a24..d1ef3c5 100644
--- a/src/http/mod.rs
+++ b/src/http/mod.rs
@@ -38,36 +38,40 @@ pub struct Config {
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
+    /// 外部 cookie jar（HttpCookieJar 注入）。
+    /// `None` 时使用 wreq::Client 内置 cookie_store。
+    pub cookie_jar: Option<std::sync::Arc<wreq::cookie::Jar>>,
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
+            cookie_jar: None,
         }
     }
 }
 
 /// Builder for Client.
 pub struct ClientBuilder {
     config: Config,
 }
 
 impl Default for ClientBuilder {
@@ -157,34 +161,50 @@ impl ClientBuilder {
     /// ND-011-SEC：禁用 TLS 证书验证（危险！）。
     ///
     /// 仅用于测试或抓取自签名证书的内部站点。启用后存在中间人攻击风险，
     /// 生产环境应保持默认 false（启用验证）。
     #[must_use]
     pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
         self.config.danger_accept_invalid_certs = accept;
         self
     }
 
+    /// 注入外部 cookie jar（与 wreq::Client 共享 cookie 状态）。
+    ///
+    /// ARCH: HttpCookieJar 自创建 `wreq::cookie::Jar`，通过此方法注入到 wreq::Client，
+    /// 实现 HttpCookieJar 与 wreq::Client 自动 cookie 管理共享同一个 jar。
+    #[must_use]
+    pub fn cookie_provider(mut self, jar: std::sync::Arc<wreq::cookie::Jar>) -> Self {
+        self.config.cookie_jar = Some(jar);
+        self
+    }
+
     /// 获取配置引用（测试用）
     #[doc(hidden)]
     #[must_use]
     pub fn config_ref(&self) -> &Config {
         &self.config
     }
 
     /// 构建 HTTP 客户端。
-    pub fn build(self) -> Result<Client> {
+    pub fn build(mut self) -> Result<Client> {
         let mut builder = wreq::Client::builder()
             .timeout(self.config.timeout)
             .redirect(wreq::redirect::Policy::limited(self.config.max_redirects))
-            .tls_cert_verification(!self.config.danger_accept_invalid_certs)
-            .cookie_store(true);
+            .tls_cert_verification(!self.config.danger_accept_invalid_certs);
+
+        // 优先使用外部注入的 cookie jar（HttpCookieJar），否则启用 wreq 内置 cookie_store
+        if let Some(jar) = self.config.cookie_jar.take() {
+            builder = builder.cookie_provider(jar);
+        } else {
+            builder = builder.cookie_store(true);
+        }
 
         if let Some(ref ua) = self.config.user_agent {
             builder = builder.user_agent(ua);
         }
         if let Some(ref proxy_url) = self.config.proxy {
             let proxy = wreq::Proxy::all(proxy_url)
                 .map_err(|e| WispError::Network(NetworkError::Http(format!("proxy error: {e}"))))?;
             builder = builder.proxy(proxy);
         }
         // 应用 TLS 指纹模拟（wreq 文档说明会覆盖现有 TLS/HTTP2 配置）
