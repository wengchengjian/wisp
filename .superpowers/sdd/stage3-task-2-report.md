# Stage 3 Task 2 报告：fetch::Client 内部类型从 reqwest 切换到 wreq

**Status:** DONE_WITH_CONCERNS

**日期：** 2026-07-21
**提交：** `7339e4e44f5ce9252049d4e2d0cae0a35637abf2`
**文件：** `src/fetch/mod.rs`（15 insertions, 15 deletions）

---

## 已实施的替换（line 号基于修改后文件）

| # | 位置 | 旧 | 新 |
|---|---|---|---|
| 1 | line 3 | `//! Wraps reqwest with builder pattern, ...` | `//! Wraps wreq with builder pattern, ...` |
| 2 | line 51 | `reqwest::Client::builder()` | `wreq::Client::builder()` |
| 3 | line 53 | `reqwest::redirect::Policy::limited(...)` | `wreq::redirect::Policy::limited(...)` |
| 4 | line 54 | `.danger_accept_invalid_certs(false)` | `.tls_cert_verification(true)` |
| 5 | line 60 | `reqwest::Proxy::all(proxy_url)` | `wreq::Proxy::all(proxy_url)` |
| 6 | line 75 | `http: reqwest::Client` | `http: wreq::Client` |
| 7 | line 101 | `reqwest::header::CONTENT_TYPE` | `wreq::header::CONTENT_TYPE` |
| 8 | line 115 | `reqwest::header::CONTENT_TYPE` | `wreq::header::CONTENT_TYPE` |
| 9 | line 131 | `reqwest::header::HeaderMap`（返回类型） | `wreq::header::HeaderMap` |
| 10 | line 132 | `reqwest::header::HeaderMap::new()` | `wreq::header::HeaderMap::new()` |
| 11 | line 135 | `reqwest::header::HeaderName::from_bytes(...)` | `wreq::header::HeaderName::from_bytes(...)` |
| 12 | line 136 | `reqwest::header::HeaderValue::from_str(...)` | `wreq::header::HeaderValue::from_str(...)` |
| 13 | line 144 | `resp: reqwest::Response` | `resp: wreq::Response` |
| 14 | line 148 | `reqwest::header::CONTENT_TYPE` | `wreq::header::CONTENT_TYPE` |
| 15 | line 146 | `resp.url().to_string()` | `resp.uri().to_string()`（API 修正，详见 Concerns） |

`#[derive(Clone)]` 在 `Client` 上保留未变 —— wreq::Client 实现了 Clone trait（arc-based），derive 直接通过，无需手动 impl。

---

## 编译结果

`cargo check`：**通过**（exit 0，3.24s）

```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.24s
```

4 个 warning 全部为预存在问题（与本次改动无关）：
- `src/browser/mod.rs:55` unused import `CommandExt`
- `src/scraper/mod.rs:185` unused variable `opts`
- `src/page/mod.rs:17` field `headless` never read
- `src/challenge/mod.rs:126` methods `wait_js_challenge` / `wait_managed` never used

---

## 测试结果

`cargo test --lib`：**35 passed, 0 failed**（与 stage 2 一致，无回归）

```
running 35 tests
...
test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

覆盖 fetch::proxy::tests（4）、parser::tests（11）、parser::difflib::tests（1）、browser::launch::tests（5）、patches::args::tests（3）、proxy::tests（3）、text::tests（5）、其他（3）。

---

## Commits

| Hash | Message |
|---|---|
| `7339e4e44f5ce9252049d4e2d0cae0a35637abf2` | `refactor: fetch::Client 内部类型从 reqwest 切换到 wreq（保持公共 API 兼容）` |

仅提交 `src/fetch/mod.rs`，未触碰其他文件。

---

## Concerns

### 1. API 差异：`Response::url()` → `Response::uri()`

任务描述中 "Verified wreq 6.0.0-rc API compatibility" 列出 `wreq::Response::url() -> &Url ✓`，但实际 wreq 6.0.0-rc.29 的 API 是 `Response::uri() -> &http::Uri`（来自 `http2` crate fork）。

**首次 `cargo check` 报错：**
```
error[E0599]: no method named `url` found for struct `wreq::Response` in the current scope
   --> src\fetch\mod.rs:146:24
    |
146 |         let url = resp.url().to_string();
    |                        ^^^
help: there is a method `uri` with a similar name
```

**修复：** 将 line 146 的 `resp.url().to_string()` 改为 `resp.uri().to_string()`。`http::Uri` 实现了 `Display` trait，`.to_string()` 行为与 `Url::to_string()` 等价（输出形如 `https://example.com/path`）。

**影响：** 仅内部实现细节。`Response::url` 字段类型仍为 `String`，公共 API 完全不变。`crawl::Engine` 和 `crawl::robots` 透明。

### 2. `#[derive(Clone)]` 保留未变

任务描述提到 "wreq::Client does NOT implement Clone directly — but wreq::Client::clone() exists"，建议可能需要手动 impl Clone。**实测 derive(Clone) 直接通过**，wreq::Client 已实现 `Clone` trait（arc-based，cheap clone）。无需手动 impl。

### 3. 公共 API 兼容性确认

- `Client` / `ClientBuilder` / `Config` / `Response` 签名未变
- `get` / `post` / `put` / `delete` / `builder` 签名未变
- `crawl::Engine` 和 `crawl::robots` 对切换透明（无需修改）
- Task 3 的 emulation / header_order 字段未引入（本任务仅做机械类型替换）

---

## 下一步

Task 3 可在当前基础上新增 `emulation: Option<Emulation>` 和 `header_order: Option<Vec<HeaderName>>` 字段。建议 Task 3 实施时再次核对 wreq-util 3.0.0-rc.14 的 `Emulation` 枚举变体名（计划中提到 `Chrome136` / `Firefox128` / `Safari18` 等）。
