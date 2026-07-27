# Task 3 Review Package

## Commits
89537b9 feat: 添加 HttpCookieJar（包装 wreq::cookie::Jar）

## Diff Stat
 src/cookie/http.rs | 252 +++++++++++++++++++++++++++++++++++++++++++++++++++++
 src/cookie/mod.rs  |   2 +
 2 files changed, 254 insertions(+)

## Full Diff
diff --git a/src/cookie/http.rs b/src/cookie/http.rs
new file mode 100644
index 0000000..1a83c3c
--- /dev/null
+++ b/src/cookie/http.rs
@@ -0,0 +1,252 @@
+//! HTTP cookie jar — 包装 wreq::cookie::Jar。
+//!
+//! ARCH: HttpCookieJar 自创建 `wreq::cookie::Jar`，通过 `ClientBuilder::cookie_provider`
+//! 注入到 wreq::Client，实现读写共享（HttpCookieJar 写入 → wreq::Client 自动携带）。
+//! wreq::Client 6.0.0-rc.29 不暴露 cookie_store getter，因此采用注入式共享。
+
+use std::sync::Arc;
+
+use async_trait::async_trait;
+use url::Url;
+
+use crate::cookie::{Cookie, CookieJar};
+
+/// HTTP cookie jar（包装 wreq::cookie::Jar）。
+pub struct HttpCookieJar {
+    jar: Arc<wreq::cookie::Jar>,
+}
+
+impl HttpCookieJar {
+    /// 创建空 jar。
+    #[must_use]
+    pub fn new() -> Self {
+        Self {
+            jar: Arc::new(wreq::cookie::Jar::default()),
+        }
+    }
+
+    /// 暴露内部 jar 供 wreq::Client::builder().cookie_provider() 使用。
+    ///
+    /// 用法：
+    /// ```ignore
+    /// let http_jar = Arc::new(HttpCookieJar::new());
+    /// let client = wreq::Client::builder()
+    ///     .cookie_provider(http_jar.jar())
+    ///     .build()?;
+    /// ```
+    #[must_use]
+    pub fn jar(&self) -> Arc<wreq::cookie::Jar> {
+        Arc::clone(&self.jar)
+    }
+}
+
+impl Default for HttpCookieJar {
+    fn default() -> Self {
+        Self::new()
+    }
+}
+
+#[async_trait]
+impl CookieJar for HttpCookieJar {
+    async fn get(&self, url: &Url) -> Vec<Cookie> {
+        // wreq::cookie::Jar 不暴露按 uri 返回 Vec<Cookie> 的 API，
+        // 使用 get_all + 手动按 domain/path 过滤，保持 trait 语义清晰。
+        let host = url.host_str().unwrap_or("");
+        let path = url.path();
+        self.jar
+            .get_all()
+            .filter(|c| {
+                let domain_match = c.domain().is_some_and(|d| host.ends_with(d));
+                let path_match = c.path().is_none_or(|p| path.starts_with(p));
+                domain_match && path_match
+            })
+            .map(|c| Cookie {
+                name: c.name().to_string(),
+                value: c.value().to_string(),
+                domain: c.domain().unwrap_or(host).to_string(),
+                path: c.path().unwrap_or("/").to_string(),
+                secure: c.secure(),
+                http_only: c.http_only(),
+                same_site: if c.same_site_lax() {
+                    Some("Lax".into())
+                } else if c.same_site_strict() {
+                    Some("Strict".into())
+                } else {
+                    None
+                },
+                expires: c.expires().map(|t| {
+                    t.duration_since(std::time::UNIX_EPOCH)
+                        .map_or(0.0, |d| d.as_secs_f64())
+                }),
+            })
+            .collect()
+    }
+
+    async fn set(&self, cookie: Cookie) {
+        // 构造 Set-Cookie 字符串注入到 wreq::cookie::Jar
+        let mut cookie_str = format!("{}={}", cookie.name, cookie.value);
+        cookie_str.push_str(&format!("; Domain={}", cookie.domain));
+        cookie_str.push_str(&format!("; Path={}", cookie.path));
+        if cookie.secure {
+            cookie_str.push_str("; Secure");
+        }
+        if cookie.http_only {
+            cookie_str.push_str("; HttpOnly");
+        }
+        if let Some(ref ss) = cookie.same_site {
+            cookie_str.push_str(&format!("; SameSite={ss}"));
+        }
+        // 使用 domain 构造关联 uri（Jar 会从中提取 host 并校验 domain-match）
+        let uri = format!("https://{}/", cookie.domain);
+        self.jar.add(cookie_str.as_str(), &uri);
+    }
+
+    async fn clear(&self, url: &Url) {
+        // wreq::cookie::Jar 没有 clear-by-url，只能全清或按 name+path 删除。
+        // 实现：收集与 url host 匹配的 cookie，用其原始 domain/path 构造精确 URI 删除。
+        let host = url.host_str().unwrap_or("");
+        let to_remove: Vec<_> = self
+            .jar
+            .get_all()
+            .filter(|c| c.domain().is_some_and(|d| host.ends_with(d)))
+            .map(|c| {
+                let domain = c.domain().unwrap_or(host).to_string();
+                let path = c.path().unwrap_or("/").to_string();
+                (c, domain, path)
+            })
+            .collect();
+        for (cookie, domain, path) in to_remove {
+            let uri = format!("https://{domain}{path}");
+            // Jar::remove 接受 Into<RawCookie<'static>>，wreq::cookie::Cookie 满足此约束
+            self.jar.remove(cookie, &uri);
+        }
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    fn make_url(s: &str) -> Url {
+        Url::parse(s).expect("合法 URL")
+    }
+
+    #[tokio::test]
+    async fn http_set_and_get_cookie() {
+        let jar = HttpCookieJar::new();
+        let cookie = Cookie {
+            name: "session".into(),
+            value: "abc".into(),
+            domain: "example.com".into(),
+            path: "/".into(),
+            secure: false,
+            http_only: false,
+            same_site: Some("Lax".into()),
+            expires: None,
+        };
+        jar.set(cookie).await;
+
+        let url = make_url("https://example.com/path");
+        let cookies = jar.get(&url).await;
+        assert_eq!(cookies.len(), 1);
+        assert_eq!(cookies[0].name, "session");
+        assert_eq!(cookies[0].value, "abc");
+    }
+
+    #[tokio::test]
+    async fn http_header_returns_string() {
+        let jar = HttpCookieJar::new();
+        jar.set(Cookie {
+            name: "a".into(),
+            value: "1".into(),
+            domain: "example.com".into(),
+            path: "/".into(),
+            secure: false,
+            http_only: false,
+            same_site: None,
+            expires: None,
+        })
+        .await;
+        jar.set(Cookie {
+            name: "b".into(),
+            value: "2".into(),
+            domain: "example.com".into(),
+            path: "/".into(),
+            secure: false,
+            http_only: false,
+            same_site: None,
+            expires: None,
+        })
+        .await;
+
+        let url = make_url("https://example.com/");
+        let header = jar.header(&url).await;
+        assert!(header.is_some());
+        let header = header.expect("非空 header");
+        assert!(header.contains("a=1"));
+        assert!(header.contains("b=2"));
+    }
+
+    #[tokio::test]
+    async fn http_clear_removes_matching() {
+        let jar = HttpCookieJar::new();
+        jar.set(Cookie {
+            name: "x".into(),
+            value: "v".into(),
+            domain: "example.com".into(),
+            path: "/".into(),
+            secure: false,
+            http_only: false,
+            same_site: None,
+            expires: None,
+        })
+        .await;
+
+        let url = make_url("https://example.com/");
+        assert!(!jar.get(&url).await.is_empty());
+
+        jar.clear(&url).await;
+        assert!(jar.get(&url).await.is_empty(), "clear 后应无 cookie");
+    }
+
+    #[tokio::test]
+    async fn http_jar_injectable_into_wreq_client() {
+        // 验证 jar() 返回的 Arc<wreq::cookie::Jar> 可注入到 wreq::Client::builder()
+        let http_jar = HttpCookieJar::new();
+        let jar = http_jar.jar();
+        let client = wreq::Client::builder().cookie_provider(jar).build();
+        assert!(client.is_ok(), "应能注入到 wreq::Client");
+    }
+
+    #[tokio::test]
+    async fn http_domain_filter() {
+        let jar = HttpCookieJar::new();
+        jar.set(Cookie {
+            name: "a".into(),
+            value: "1".into(),
+            domain: "example.com".into(),
+            path: "/".into(),
+            secure: false,
+            http_only: false,
+            same_site: None,
+            expires: None,
+        })
+        .await;
+        jar.set(Cookie {
+            name: "b".into(),
+            value: "2".into(),
+            domain: "other.com".into(),
+            path: "/".into(),
+            secure: false,
+            http_only: false,
+            same_site: None,
+            expires: None,
+        })
+        .await;
+
+        let url = make_url("https://example.com/");
+        let cookies = jar.get(&url).await;
+        assert_eq!(cookies.len(), 1);
+        assert_eq!(cookies[0].name, "a");
+    }
+}
diff --git a/src/cookie/mod.rs b/src/cookie/mod.rs
index 3f90708..881efaf 100644
--- a/src/cookie/mod.rs
+++ b/src/cookie/mod.rs
@@ -4,22 +4,24 @@
 //! strategy 可访问。三种实现：
 //! - HttpCookieJar: 包装 wreq::cookie::Jar（与 wreq::Client 共享）
 //! - BrowserCookieJar: 通过 CDP Network.getCookies/setCookie
 //! - CfCookieJar: moka::Cache + 文件持久化（从 FetchClient 迁出）
 
 use async_trait::async_trait;
 use serde::{Deserialize, Serialize};
 use url::Url;
 
 pub mod cf;
+pub mod http;
 
 pub use cf::{CfCookieJar, CfSession};
+pub use http::HttpCookieJar;
 
 /// Cookie 表示（统一格式，跨 HTTP/浏览器/CF）。
 #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
 pub struct Cookie {
     /// Cookie 名称。
     pub name: String,
     /// Cookie 值。
     pub value: String,
     /// Cookie 作用域名（如 "example.com"）。
     pub domain: String,
