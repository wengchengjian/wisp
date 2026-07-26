# Task 3 报告：HttpCookieJar 实现（包装 wreq::cookie::Jar）

## 实施摘要

按 plan 5 步 TDD 完成 `HttpCookieJar`，包装 `wreq::cookie::Jar`，通过 `ClientBuilder::cookie_provider` 注入到 wreq::Client。

### 文件变更
- **新增** `/home/weng/wisp/src/cookie/http.rs`（254 行）：`HttpCookieJar` 结构 + `CookieJar` trait 实现 + 5 个内联测试
- **修改** `/home/weng/wisp/src/cookie/mod.rs`：添加 `pub mod http;` 和 `pub use http::HttpCookieJar;`

### 核心实现
- `HttpCookieJar::new()`：创建空 jar，内部 `Arc<wreq::cookie::Jar::default()>`
- `HttpCookieJar::jar()`：返回 `Arc<wreq::cookie::Jar>` 供 `wreq::Client::builder().cookie_provider()` 使用
- `CookieJar::get`：`jar.get_all()` + 按 domain/path 手动过滤，映射到统一 `Cookie` 结构
- `CookieJar::set`：构造 Set-Cookie 字符串 + `https://{domain}/` URI，调用 `jar.add()`
- `CookieJar::clear`：收集 host 匹配的 cookie，用其原始 domain/path 构造精确 URI 调用 `jar.remove()`

## 测试结果

```
cargo test --lib cookie::http::tests
test result: ok. 5 passed; 0 failed; 0 ignored
```

5 个测试：
- `http_set_and_get_cookie` ✓
- `http_header_returns_string` ✓
- `http_clear_removes_matching` ✓
- `http_jar_injectable_into_wreq_client` ✓（验证 `cookie_provider` 注入成功）
- `http_domain_filter` ✓

### 全量回归
```
cargo test --lib
test result: ok. 303 passed; 0 failed; 8 ignored
```
（含 5 个新 http 测试 + 298 个原有测试，Task 1-2 的 mock/cf 测试全绿）

### clippy
`cargo clippy --lib --no-deps` 干净（0 warning），项目用 `clippy::pedantic`。

## Commit

- Hash: `89537b9`
- Message: `feat: 添加 HttpCookieJar（包装 wreq::cookie::Jar）`
- Parent: `408d6db`（Task 1-2 HEAD）

## 自审发现（与 plan 的偏差）

### 1. plan 中 `clear` 方法使用了不公开的 `wreq::cookie::RawCookie`

**问题**：plan 的 `clear` 调用 `wreq::cookie::RawCookie::build(name).to_string()`，但 `RawCookie` 只是 wreq 内部的 `use cookie::{Cookie as RawCookie, ...}` 别名，**未公开导出**。`wreq::cookie` 模块只公开 `Cookie<'a>` 包装结构和 `Jar`、`CookieStore`、`IntoCookie` 等。

**修复**：改用 `jar.get_all()` 返回的 `wreq::cookie::Cookie<'static>`（包装了 `RawCookie`），直接传给 `jar.remove(cookie, &uri)`。`Jar::remove<C, U>` 要求 `C: Into<RawCookie<'static>>`，而 wreq 已为 `Cookie<'c>` 实现 `From<Cookie<'c>> for RawCookie<'c>`，满足约束。

**改进**：用每个 cookie 的原始 domain/path 构造精确 URI（`https://{domain}{path}`），而非统一用入参 URL。这样能删除该 domain 下所有路径的 cookie，而不只限于入参 URL 的精确 path。plan 原方案受限于 `Jar::remove` 按 `uri.path()` 查找，只能删除入参 URL path 下的条目。

### 2. plan 中 `get` 方法有死代码

**问题**：plan 的 `get` 方法开头转换 `let uri: wreq::Uri = match url.as_str().try_into() {...}`，但 `uri` 变量后续未使用（实际用 `url.host_str()` 和 `url.path()`）。

**修复**：删除未使用的 `uri` 绑定，避免 warning。

### 3. clippy pedantic 修复

plan 代码触发两个 pedantic warning，已修复：
- `c.path().map_or(true, |p| ...)` → `c.path().is_none_or(|p| ...)`（`unnecessary_map_or`）
- `.map(|d| ...).unwrap_or(0.0)` → `.map_or(0.0, |d| ...)`（`map_unwrap_or`）

## 状态

✅ **完成**。Task 3 已实施并通过验证，可进入 Task 4（BrowserCookieJar via CDP）。

---

## 修复（Important #1）：set 方法丢弃 expires 字段

### 问题

审查发现 `HttpCookieJar::set` 构造 Set-Cookie 字符串时未写入 `Max-Age` 或 `Expires`，导致带 `expires` 的 cookie 被当作 session cookie 存储，信息丢失。

### 根因分析（与任务描述的偏离）

任务描述建议追加 `; Max-Age={max_age}`（max_age = expires - now）。但实证验证发现该方案在 `wreq 6.0.0-rc.29` + `cookie 0.18.1` 下**无效**：

- `cookie::Cookie` 把 `Max-Age` 和 `Expires` 存为**独立字段**（`max_age: Option<Duration>` vs `expires: Option<Expiration>`）
- `wreq::cookie::Cookie::expires()` 实现（`src/cookie.rs:161-166`）**只读 `expires` 字段**，不读 `max_age`
- 因此 `Max-Age=3600` 会被 parse 填入 `max_age` 字段，但 `cookies[0].expires` 读回仍是 `None`

调试输出验证：
```
DEBUG cookies = [Cookie { ..., expires: None }]
```

### 修复方案

改用 `Expires=<HTTP-date>`（RFC 7231 IMF-fixdate 格式，如 `Wed, 21 Oct 2025 07:28:00 GMT`）。`cookie::Cookie::parse` 会把它解析到 `expires` 字段（`Expiration::DateTime`），从而被 `wreq::cookie::Cookie::expires()` 正确读回。

### 实现要点

`src/cookie/http.rs` 的 `set` 方法：

1. 取当前 Unix 时间戳 `now`
2. match `cookie.expires`：
   - `Some(expires) if expires > now` → 用 `chrono::DateTime::<Utc>::from_timestamp(expires as i64, 0)` 转 `DateTime`，format 成 `"%a, %d %b %Y %H:%M:%S GMT"`
   - `Some(_)` → 已过期，`return` 跳过不写入
   - `None` → session cookie，正常写入不带 Expires
3. 在 Set-Cookie 字符串末尾追加 `; Expires={http_date}`

### 新增测试

1. `http_set_with_expires`：写入 `expires = now + 3600`，读回 cookies[0].expires 应为 Some 且与入参偏差 < 5s
2. `http_set_with_expired_expires_skipped`：写入 `expires = now - 10`（已过期），读回应为空

### 验证

```
cargo test --lib cookie::http
test result: ok. 7 passed; 0 failed; 0 ignored

cargo test --lib
test result: ok. 305 passed; 0 failed; 8 ignored
（原 303 + 新增 2 个测试）

cargo clippy --lib -- -D warnings
Finished, 0 warning
```

### Commit

- Hash: `f5d5bf1`
- Message: `fix: HttpCookieJar::set 写入 Expires 字段避免 expires 丢失`
- Parent: `89537b9`（Task 3 原 HEAD）

