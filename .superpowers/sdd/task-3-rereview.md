# Task 3 Re-Review Package (after fix)

## Commits
54aeb4c fix: HttpCookieJar::set 写入 Expires 字段避免 expires 丢失

## Diff Stat
 .superpowers/sdd/task-3-report.md | 134 ++++++++++++++++++++++++++++++++++++++
 src/cookie/http.rs                |  87 +++++++++++++++++++++++++
 2 files changed, 221 insertions(+)

## Full Diff
diff --git a/.superpowers/sdd/task-3-report.md b/.superpowers/sdd/task-3-report.md
new file mode 100644
index 0000000..ff3b064
--- /dev/null
+++ b/.superpowers/sdd/task-3-report.md
@@ -0,0 +1,134 @@
+# Task 3 报告：HttpCookieJar 实现（包装 wreq::cookie::Jar）
+
+## 实施摘要
+
+按 plan 5 步 TDD 完成 `HttpCookieJar`，包装 `wreq::cookie::Jar`，通过 `ClientBuilder::cookie_provider` 注入到 wreq::Client。
+
+### 文件变更
+- **新增** `/home/weng/wisp/src/cookie/http.rs`（254 行）：`HttpCookieJar` 结构 + `CookieJar` trait 实现 + 5 个内联测试
+- **修改** `/home/weng/wisp/src/cookie/mod.rs`：添加 `pub mod http;` 和 `pub use http::HttpCookieJar;`
+
+### 核心实现
+- `HttpCookieJar::new()`：创建空 jar，内部 `Arc<wreq::cookie::Jar::default()>`
+- `HttpCookieJar::jar()`：返回 `Arc<wreq::cookie::Jar>` 供 `wreq::Client::builder().cookie_provider()` 使用
+- `CookieJar::get`：`jar.get_all()` + 按 domain/path 手动过滤，映射到统一 `Cookie` 结构
+- `CookieJar::set`：构造 Set-Cookie 字符串 + `https://{domain}/` URI，调用 `jar.add()`
+- `CookieJar::clear`：收集 host 匹配的 cookie，用其原始 domain/path 构造精确 URI 调用 `jar.remove()`
+
+## 测试结果
+
+```
+cargo test --lib cookie::http::tests
+test result: ok. 5 passed; 0 failed; 0 ignored
+```
+
+5 个测试：
+- `http_set_and_get_cookie` ✓
+- `http_header_returns_string` ✓
+- `http_clear_removes_matching` ✓
+- `http_jar_injectable_into_wreq_client` ✓（验证 `cookie_provider` 注入成功）
+- `http_domain_filter` ✓
+
+### 全量回归
+```
+cargo test --lib
+test result: ok. 303 passed; 0 failed; 8 ignored
+```
+（含 5 个新 http 测试 + 298 个原有测试，Task 1-2 的 mock/cf 测试全绿）
+
+### clippy
+`cargo clippy --lib --no-deps` 干净（0 warning），项目用 `clippy::pedantic`。
+
+## Commit
+
+- Hash: `89537b9`
+- Message: `feat: 添加 HttpCookieJar（包装 wreq::cookie::Jar）`
+- Parent: `408d6db`（Task 1-2 HEAD）
+
+## 自审发现（与 plan 的偏差）
+
+### 1. plan 中 `clear` 方法使用了不公开的 `wreq::cookie::RawCookie`
+
+**问题**：plan 的 `clear` 调用 `wreq::cookie::RawCookie::build(name).to_string()`，但 `RawCookie` 只是 wreq 内部的 `use cookie::{Cookie as RawCookie, ...}` 别名，**未公开导出**。`wreq::cookie` 模块只公开 `Cookie<'a>` 包装结构和 `Jar`、`CookieStore`、`IntoCookie` 等。
+
+**修复**：改用 `jar.get_all()` 返回的 `wreq::cookie::Cookie<'static>`（包装了 `RawCookie`），直接传给 `jar.remove(cookie, &uri)`。`Jar::remove<C, U>` 要求 `C: Into<RawCookie<'static>>`，而 wreq 已为 `Cookie<'c>` 实现 `From<Cookie<'c>> for RawCookie<'c>`，满足约束。
+
+**改进**：用每个 cookie 的原始 domain/path 构造精确 URI（`https://{domain}{path}`），而非统一用入参 URL。这样能删除该 domain 下所有路径的 cookie，而不只限于入参 URL 的精确 path。plan 原方案受限于 `Jar::remove` 按 `uri.path()` 查找，只能删除入参 URL path 下的条目。
+
+### 2. plan 中 `get` 方法有死代码
+
+**问题**：plan 的 `get` 方法开头转换 `let uri: wreq::Uri = match url.as_str().try_into() {...}`，但 `uri` 变量后续未使用（实际用 `url.host_str()` 和 `url.path()`）。
+
+**修复**：删除未使用的 `uri` 绑定，避免 warning。
+
+### 3. clippy pedantic 修复
+
+plan 代码触发两个 pedantic warning，已修复：
+- `c.path().map_or(true, |p| ...)` → `c.path().is_none_or(|p| ...)`（`unnecessary_map_or`）
+- `.map(|d| ...).unwrap_or(0.0)` → `.map_or(0.0, |d| ...)`（`map_unwrap_or`）
+
+## 状态
+
+✅ **完成**。Task 3 已实施并通过验证，可进入 Task 4（BrowserCookieJar via CDP）。
+
+---
+
+## 修复（Important #1）：set 方法丢弃 expires 字段
+
+### 问题
+
+审查发现 `HttpCookieJar::set` 构造 Set-Cookie 字符串时未写入 `Max-Age` 或 `Expires`，导致带 `expires` 的 cookie 被当作 session cookie 存储，信息丢失。
+
+### 根因分析（与任务描述的偏离）
+
+任务描述建议追加 `; Max-Age={max_age}`（max_age = expires - now）。但实证验证发现该方案在 `wreq 6.0.0-rc.29` + `cookie 0.18.1` 下**无效**：
+
+- `cookie::Cookie` 把 `Max-Age` 和 `Expires` 存为**独立字段**（`max_age: Option<Duration>` vs `expires: Option<Expiration>`）
+- `wreq::cookie::Cookie::expires()` 实现（`src/cookie.rs:161-166`）**只读 `expires` 字段**，不读 `max_age`
+- 因此 `Max-Age=3600` 会被 parse 填入 `max_age` 字段，但 `cookies[0].expires` 读回仍是 `None`
+
+调试输出验证：
+```
+DEBUG cookies = [Cookie { ..., expires: None }]
+```
+
+### 修复方案
+
+改用 `Expires=<HTTP-date>`（RFC 7231 IMF-fixdate 格式，如 `Wed, 21 Oct 2025 07:28:00 GMT`）。`cookie::Cookie::parse` 会把它解析到 `expires` 字段（`Expiration::DateTime`），从而被 `wreq::cookie::Cookie::expires()` 正确读回。
+
+### 实现要点
+
+`src/cookie/http.rs` 的 `set` 方法：
+
+1. 取当前 Unix 时间戳 `now`
+2. match `cookie.expires`：
+   - `Some(expires) if expires > now` → 用 `chrono::DateTime::<Utc>::from_timestamp(expires as i64, 0)` 转 `DateTime`，format 成 `"%a, %d %b %Y %H:%M:%S GMT"`
+   - `Some(_)` → 已过期，`return` 跳过不写入
+   - `None` → session cookie，正常写入不带 Expires
+3. 在 Set-Cookie 字符串末尾追加 `; Expires={http_date}`
+
+### 新增测试
+
+1. `http_set_with_expires`：写入 `expires = now + 3600`，读回 cookies[0].expires 应为 Some 且与入参偏差 < 5s
+2. `http_set_with_expired_expires_skipped`：写入 `expires = now - 10`（已过期），读回应为空
+
+### 验证
+
+```
+cargo test --lib cookie::http
+test result: ok. 7 passed; 0 failed; 0 ignored
+
+cargo test --lib
+test result: ok. 305 passed; 0 failed; 8 ignored
+（原 303 + 新增 2 个测试）
+
+cargo clippy --lib -- -D warnings
+Finished, 0 warning
+```
+
+### Commit
+
+- Hash: `f5d5bf1`
+- Message: `fix: HttpCookieJar::set 写入 Expires 字段避免 expires 丢失`
+- Parent: `89537b9`（Task 3 原 HEAD）
+
diff --git a/src/cookie/http.rs b/src/cookie/http.rs
index 1a83c3c..fd3709a 100644
--- a/src/cookie/http.rs
+++ b/src/cookie/http.rs
@@ -76,33 +76,53 @@ impl CookieJar for HttpCookieJar {
                 },
                 expires: c.expires().map(|t| {
                     t.duration_since(std::time::UNIX_EPOCH)
                         .map_or(0.0, |d| d.as_secs_f64())
                 }),
             })
             .collect()
     }
 
     async fn set(&self, cookie: Cookie) {
+        // expires 是 Unix 时间戳（秒）。wreq::cookie::Jar 的 Cookie::expires() 仅读
+        // cookie::Cookie 的 expires 字段（Expiration 类型），不读 max_age 字段，
+        // 因此用 `Expires=<HTTP-date>` 而非 `Max-Age=<secs>`，否则 expires 信息丢失。
+        // 过期 cookie（expires <= now）直接跳过。
+        let now = std::time::SystemTime::now()
+            .duration_since(std::time::UNIX_EPOCH)
+            .map_or(0.0, |d| d.as_secs_f64());
+        let expires_str = match cookie.expires {
+            Some(expires) if expires > now => {
+                // RFC 7231 IMF-fixdate: "Wed, 21 Oct 2015 07:28:00 GMT"
+                chrono::DateTime::<chrono::Utc>::from_timestamp(expires as i64, 0)
+                    .map(|dt| dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
+            }
+            Some(_) => return, // 已过期
+            None => None,      // session cookie
+        };
+
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
+        if let Some(ref exp) = expires_str {
+            cookie_str.push_str(&format!("; Expires={exp}"));
+        }
         // 使用 domain 构造关联 uri（Jar 会从中提取 host 并校验 domain-match）
         let uri = format!("https://{}/", cookie.domain);
         self.jar.add(cookie_str.as_str(), &uri);
     }
 
     async fn clear(&self, url: &Url) {
         // wreq::cookie::Jar 没有 clear-by-url，只能全清或按 name+path 删除。
         // 实现：收集与 url host 匹配的 cookie，用其原始 domain/path 构造精确 URI 删除。
         let host = url.host_str().unwrap_or("");
         let to_remove: Vec<_> = self
@@ -242,11 +262,78 @@ mod tests {
             same_site: None,
             expires: None,
         })
         .await;
 
         let url = make_url("https://example.com/");
         let cookies = jar.get(&url).await;
         assert_eq!(cookies.len(), 1);
         assert_eq!(cookies[0].name, "a");
     }
+
+    #[tokio::test]
+    async fn http_set_with_expires() {
+        // 验证带 expires 的 cookie 能被正确存储并读回 expires 字段
+        let now = std::time::SystemTime::now()
+            .duration_since(std::time::UNIX_EPOCH)
+            .expect("系统时间在 1970 之后")
+            .as_secs_f64();
+        let jar = HttpCookieJar::new();
+        // expires 设为 now + 3600s（1 小时后）
+        jar.set(Cookie {
+            name: "token".into(),
+            value: "xyz".into(),
+            domain: "example.com".into(),
+            path: "/".into(),
+            secure: false,
+            http_only: true,
+            same_site: Some("Lax".into()),
+            expires: Some(now + 3600.0),
+        })
+        .await;
+
+        let url = make_url("https://example.com/");
+        let cookies = jar.get(&url).await;
+        assert_eq!(cookies.len(), 1, "应读回 1 个 cookie");
+        assert_eq!(cookies[0].name, "token");
+        assert_eq!(cookies[0].value, "xyz");
+        // 读回的 expires 应为 Some 且接近入参（差异在 5 秒内视为精度可接受）
+        let read_expires = cookies[0]
+            .expires
+            .expect("expires 应为 Some，非 session cookie");
+        let delta = (read_expires - (now + 3600.0)).abs();
+        assert!(
+            delta < 5.0,
+            "expires 偏差过大: {delta}s（读回 {read_expires}, 期望 {}）",
+            now + 3600.0
+        );
+    }
+
+    #[tokio::test]
+    async fn http_set_with_expired_expires_skipped() {
+        // 过期 cookie（expires <= now）应被跳过：jar 不存储，读回为空
+        let now = std::time::SystemTime::now()
+            .duration_since(std::time::UNIX_EPOCH)
+            .expect("系统时间在 1970 之后")
+            .as_secs_f64();
+        let jar = HttpCookieJar::new();
+        jar.set(Cookie {
+            name: "dead".into(),
+            value: "v".into(),
+            domain: "example.com".into(),
+            path: "/".into(),
+            secure: false,
+            http_only: false,
+            same_site: None,
+            expires: Some(now - 10.0), // 已过期 10 秒
+        })
+        .await;
+
+        let url = make_url("https://example.com/");
+        let cookies = jar.get(&url).await;
+        assert!(
+            cookies.is_empty(),
+            "过期 cookie 不应被存储，但读回 {len} 条",
+            len = cookies.len()
+        );
+    }
 }
