# SitemapSpider 迁移 + SessionManager 删除 + 冗余代码清理

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 删除设计错误的 SessionManager 模块（无业界先例，多 fetcher 路由被 auto_rules 取代），SitemapSpider 迁移为 SpiderBuilder 预设，清理全部冗余代码。

**Tech Stack:** Rust, tokio, async-trait

---

## 背景

### 调研结论
1. **Crawlee SessionPool**：是身份池（cookie/UA/代理轮换反封锁），框架自动注入，用户不写路由
2. **Scrapy cookiejar**：用户在 meta 标记，框架按标记分桶
3. **wisp SessionManager**：独创"多 fetcher 类型路由"，无业界先例，设计错误
4. **wisp 已有 auto_rules**：FetchMode::Auto 自动升级 HTTP→Stealth，已覆盖多 fetcher 需求

### 删除理由
- SessionManager 的"列表用 HTTP、详情用 Stealth"场景已被 `auto_rules` 自动升级取代
- 用户实现的 `session_for(req)` 路由既无 Crawlee 自动轮换便利，又无 Scrapy 显式标记清晰
- 整模块与 Engine 完全断开，纯死代码

### 设计原则
- **不向后兼容**：删除 parse_fn/async_parse_fn，统一到 on() API
- **删除死代码**：SessionManager 整模块 + templates.rs 整文件 + 7 个 warnings
- **保留 auto_rules**：自动模式升级是 wisp 已有的正确解法

---

## 文件结构

| 文件 | 责任 | 动作 |
|------|------|------|
| `src/crawl/mod.rs` | 删 `pub mod session;`；Spider trait 删 4 死方法 | 修改 |
| `src/crawl/session.rs` | 整文件删除 | **删除** |
| `src/crawl/builder.rs` | 删 parse_fn/async_parse_fn；加 sitemap() 预设 | 修改 |
| `src/crawl/templates.rs` | 整文件删除 | **删除** |
| `src/stealth/challenge.rs` | 删 wait_js_challenge / wait_managed | 修改 |
| `src/browser/page.rs` | 删 headless 字段 | 修改 |
| `src/browser/mod.rs` | 删 unused CommandExt import | 修改 |
| `src/fetcher/mod.rs` | 删 unused imports | 修改 |
| `src/fetcher/session.rs` | 删 unused mut | 修改 |
| `src/crawl/engine.rs` | 修复 final_resp 冗余赋值 | 修改 |
| `tests/builder_api_test.rs` | 删 parse_fn 测试，改用 on() | 修改 |
| `tests/session_test.rs` | 删除（SessionManager 已删） | **删除** |
| `tests/sitemap_test.rs` | 新建：测试 SpiderBuilder::sitemap() | **新建** |

---

## Task 1: 删除 SessionManager 模块

**Files:**
- Delete: `src/crawl/session.rs`
- Modify: `src/crawl/mod.rs`
- Delete: `tests/session_test.rs`

- [ ] **Step 1: 删除 session.rs 文件**

用 DeleteFile 工具删除 `src/crawl/session.rs`。

- [ ] **Step 2: 删除 mod.rs 的 session 声明**

删除 `pub mod session;` 行。
删除 `pub use session::{SessionManager, FetcherType, request_with_session, session_id_of};`（如存在 re-export）。

- [ ] **Step 3: 删除 Spider trait 的 4 个死方法**

删除：
- `fn configure_sessions(&self, _mgr: &mut session::SessionManager) {}`
- `fn session_for(&self, _req: &SpiderRequest) -> &str { "default" }`
- `fn concurrent_requests(&self) -> u32 { 8 }`（Engine 用 EngineBuilder::max_concurrent）
- `fn rotate_ua(&self) -> bool { false }`（Engine 用 http::Config.rotate_ua）

- [ ] **Step 4: 删除 session_test.rs**

用 DeleteFile 工具删除 `tests/session_test.rs`。

- [ ] **Step 5: 搜索其他引用**

用 Grep 搜索 `SessionManager|FetcherType|configure_sessions|session_for|request_with_session|session_id_of` 在 src/ 和 tests/ 中，清理所有引用。

- [ ] **Step 6: 验证编译**

```
cargo build --lib
```

- [ ] **Step 7: 提交**

```bash
git add src/crawl/mod.rs
git rm src/crawl/session.rs tests/session_test.rs
git commit -m "refactor(crawl): 删除 SessionManager 整模块" -m "设计错误：多 fetcher 路由无业界先例，被 auto_rules 取代" -m "删除 Spider trait 的 configure_sessions/session_for/concurrent_requests/rotate_ua 4 个死方法" -m "未来反封锁需求应借鉴 Crawlee 身份池模式（框架自动注入）"
```

---

## Task 2: SitemapSpider 迁移为 SpiderBuilder 预设

**Files:**
- Modify: `src/crawl/builder.rs`
- Delete: `src/crawl/templates.rs`
- Modify: `src/crawl/mod.rs`（删 `pub mod templates;`）
- New: `tests/sitemap_test.rs`

- [ ] **Step 1: SpiderBuilder 加 sitemap() 预设**

在 `src/crawl/builder.rs` 的 `impl SpiderBuilder` 中加：

```rust
/// 预设：Sitemap 爬虫。
///
/// 自动解析 sitemap.xml，提取 `<loc>` URL，follow 到指定 label 的 handler。
///
/// # 示例
/// ```ignore
/// let spider = SpiderBuilder::sitemap("my_spider", vec!["https://x.com/sitemap.xml".into()], "content")
///     .on("content", |resp| async move {
///         (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
///     })
///     .build();
/// ```
pub fn sitemap(name: &str, sitemap_urls: Vec<String>, content_label: &str) -> Self {
    let label = content_label.to_string();
    SpiderBuilder::new(name)
        .start_urls(sitemap_urls)
        .on("default", move |resp| {
            let label = label.clone();
            async move {
                let text = resp.text().unwrap_or_default();
                let re = regex::Regex::new(r"<loc>\s*(.*?)\s*</loc>").unwrap();
                let follows: Vec<SpiderRequest> = re.captures_iter(&text)
                    .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
                    .filter(|u| !u.is_empty())
                    .map(|url| SpiderRequest::get(&url).with_callback(&label))
                    .collect();
                (vec![], follows)
            }
        })
}
```

- [ ] **Step 2: 删除 templates.rs**

用 DeleteFile 工具删除 `src/crawl/templates.rs`。

- [ ] **Step 3: 删除 mod.rs 的 templates 声明**

删除 `pub mod templates;` 行。

- [ ] **Step 4: 写测试**

新建 `tests/sitemap_test.rs`：

```rust
//! SpiderBuilder::sitemap() 测试。
use wisp::crawl::*;

#[test]
fn test_sitemap_builder_creates_spider() {
    let spider = SpiderBuilder::sitemap("test", vec!["https://example.com/sitemap.xml".into()], "content")
        .on("content", |_resp| async move {
            (vec![serde_json::json!({"ok": true})], vec![])
        })
        .build();
    assert_eq!(spider.name(), "test");
    assert_eq!(spider.start_urls(), vec!["https://example.com/sitemap.xml"]);
}

#[tokio::test]
async fn test_sitemap_parses_loc_urls() {
    let spider = SpiderBuilder::sitemap("test", vec!["https://example.com/sitemap.xml".into()], "content")
        .on("content", |_resp| async move { (vec![], vec![]) })
        .build();

    let sitemap_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset>
  <url><loc>https://example.com/page1</loc></url>
  <url><loc>https://example.com/page2</loc></url>
</urlset>"#;
    let resp = SpiderResponse {
        url: "https://example.com/sitemap.xml".into(),
        status: 200,
        headers: Default::default(),
        body: sitemap_xml.as_bytes().to_vec(),
        request: SpiderRequest::get("https://example.com/sitemap.xml"),
        tracker: None,
        from_cache: false,
    };

    let (items, follows) = spider.handle(resp).await;
    assert!(items.is_empty());
    assert_eq!(follows.len(), 2);
    assert_eq!(follows[0].url, "https://example.com/page1");
    assert_eq!(follows[1].url, "https://example.com/page2");
    assert_eq!(follows[0].callback.as_deref(), Some("content"));
}
```

- [ ] **Step 5: 验证**

```
cargo build --lib
cargo test --test sitemap_test -- --nocapture
```

- [ ] **Step 6: 提交**

```bash
git add src/crawl/builder.rs tests/sitemap_test.rs
git rm src/crawl/templates.rs
git add src/crawl/mod.rs
git commit -m "feat(builder): SpiderBuilder::sitemap() 预设替代 templates.rs" -m "SitemapSpider 迁移为 SpiderBuilder 预设方法" -m "删除 templates.rs 整个文件（死代码）"
```

---

## Task 3: 删除 parse_fn/async_parse_fn 冗余

**Files:**
- Modify: `src/crawl/builder.rs`
- Modify: `tests/builder_api_test.rs`
- Modify: `tests/callback_routing_test.rs`（如有 parse_fn 用法）

- [ ] **Step 1: 删除 ParseFn / AsyncParseFn 类型**

删除 `builder.rs` 中的：
```rust
pub type ParseFn = Box<dyn Fn(SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) + Send + Sync + 'static>;
pub type AsyncParseFn = Box<dyn Fn(SpiderResponse) -> std::pin::Pin<Box<dyn std::future::Future<Output = (Vec<Value>, Vec<SpiderRequest>)> + Send>> + Send + Sync + 'static>;
```

- [ ] **Step 2: 删除 SpiderBuilder 的 parse_fn/async_parse_fn 字段和 builder 方法**

删除：
- `parse_fn: Option<ParseFn>` 字段
- `async_parse_fn: Option<AsyncParseFn>` 字段
- `pub fn parse<F>(self, f: F) -> Self` 方法
- `pub fn parse_async<F, Fut>(self, f: F) -> Self` 方法

- [ ] **Step 3: 删除 ClosureSpider 的 parse_fn/async_parse_fn 字段**

- [ ] **Step 4: 改造 ClosureSpider::parse()**

```rust
/// parse 兜底：handle() 无匹配 handler 时调用。
/// 统一到 on() API 后，parse() 只返回空。
async fn parse(&self, _response: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
    (vec![], vec![])
}
```

- [ ] **Step 5: 更新 build() 断言**

```rust
pub fn build(self) -> ClosureSpider {
    assert!(
        !self.handlers.is_empty(),
        "SpiderBuilder: 必须注册至少一个 handler（用 .on(label, handler)）"
    );
    // ...
}
```

- [ ] **Step 6: 迁移现有测试**

将 `tests/builder_api_test.rs` 中所有 `.parse(|resp| { ... })` 改为 `.on("default", |resp| async move { ... })`。
将 `tests/callback_routing_test.rs` 中 `.parse(|_resp| { ... })` 改为 `.on("default", |_resp| async move { ... })`。
将 `builder.rs` 内联测试同样迁移。

- [ ] **Step 7: 搜索其他 parse() 用法**

用 Grep 搜索 `.parse(|` 和 `.parse_async(|` 在 src/ 和 tests/ 中的用法，全部迁移。

- [ ] **Step 8: 验证**

```
cargo build --lib
cargo test --lib crawl::builder -- --nocapture
cargo test --test builder_api_test -- --nocapture
cargo test --test callback_routing_test -- --nocapture
```

- [ ] **Step 9: 提交**

```bash
git add src/crawl/builder.rs tests/builder_api_test.rs tests/callback_routing_test.rs
git commit -m "refactor(builder): 删除 parse_fn/async_parse_fn，统一到 on() API" -m "ParseFn/AsyncParseFn 类型删除" -m "parse()/parse_async() builder 方法删除" -m "所有测试迁移到 on(label, handler)"
```

---

## Task 4: 清理 7 个 dead_code warnings

**Files:**
- Modify: `src/stealth/challenge.rs`
- Modify: `src/browser/page.rs`
- Modify: `src/browser/mod.rs`
- Modify: `src/fetcher/mod.rs`
- Modify: `src/fetcher/session.rs`
- Modify: `src/crawl/engine.rs`

- [ ] **Step 1: 删除 challenge.rs 的 wait_js_challenge / wait_managed**

删除 `src/stealth/challenge.rs` 中 `wait_js_challenge` 和 `wait_managed` 两个私有方法（约 45 行）。

- [ ] **Step 2: 删除 Page.headless 字段**

`src/browser/page.rs`：
- 删除 `pub(crate) headless: bool,` 字段
- `Page::create` 中删除 `headless` 参数的存储（参数保留用于逻辑判断）
- 删除 `let page = Self { session, session_id, frame_id, headless };` 中的 headless

- [ ] **Step 3: 删除 browser/mod.rs 的 CommandExt import**

删除 `#[cfg(windows)] use std::os::windows::process::CommandExt;`（tokio 的 creation_flags 是固有方法）。

- [ ] **Step 4: 删除 fetcher/mod.rs 的 unused imports**

删除 `use serde_json::Value;`，`WispError` 从 `use crate::error::{WispError, Result};` 中移除（改为 `use crate::error::Result;`）。

- [ ] **Step 5: 删除 fetcher/session.rs 的 unused mut**

`let mut resp` → `let resp`。

- [ ] **Step 6: 修复 engine.rs:148 冗余赋值**

`let mut final_resp: Option<SpiderResponse> = None;` → `let mut final_resp: Option<SpiderResponse>;`

- [ ] **Step 7: 验证零 warnings**

```
cargo build --lib 2>&1 | Select-String "warning"
```

预期：0 warnings。

- [ ] **Step 8: 验证测试**

```
cargo test --lib
cargo test
```

- [ ] **Step 9: 提交**

```bash
git add src/stealth/challenge.rs src/browser/page.rs src/browser/mod.rs src/fetcher/mod.rs src/fetcher/session.rs src/crawl/engine.rs
git commit -m "chore: 清理 7 个 dead_code warnings" -m "删除 wait_js_challenge/wait_managed 死方法" -m "删除 Page.headless 死字段" -m "删除 unused imports/mut，修复 final_resp 冗余赋值"
```

---

## Task 5: 全量验证

- [ ] **Step 1: 编译验证**

```
cargo build
cargo build --release
```

- [ ] **Step 2: 零 warnings 验证**

```
cargo build --lib 2>&1 | Select-String "warning"
```

预期：0 warnings。

- [ ] **Step 3: 测试验证**

```
cargo test
```

- [ ] **Step 4: 检查 session.rs 已删除**

确认 `src/crawl/session.rs` 不存在。
确认 `tests/session_test.rs` 不存在。

- [ ] **Step 5: 检查 templates.rs 已删除**

确认 `src/crawl/templates.rs` 不存在。

- [ ] **Step 6: 检查无 parse_fn 残留**

用 Grep 搜索 `ParseFn|parse_fn|async_parse_fn|\.parse\(\|` 在 src/ 和 tests/ 中，应为 0。

- [ ] **Step 7: 检查无 SessionManager 残留**

用 Grep 搜索 `SessionManager|FetcherType|configure_sessions|session_for` 在 src/ 和 tests/ 中，应为 0。

- [ ] **Step 8: git 状态确认**

```
git status
git log --oneline -6
```

---

## 使用示例

### 示例 1：Sitemap 爬虫

```rust
let spider = SpiderBuilder::sitemap("my_spider", vec!["https://example.com/sitemap.xml".into()], "content")
    .on("content", |resp| async move {
        (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
    })
    .build();

let engine = Engine::infra().build()?;
let (stats, items) = engine.run(spider).await?;
```

### 示例 2：简单爬虫（统一 on() API）

```rust
let spider = SpiderBuilder::new("simple")
    .start_urls(vec!["https://example.com/".into()])
    .on("default", |resp| async move {
        (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
    })
    .build();
```

### 示例 3：列表→详情→内容 三阶段

```rust
let spider = SpiderBuilder::new("pipeline")
    .start_urls(vec!["https://example.com/list".into()])
    .on("default", |resp| async move {
        let follows: Vec<_> = resp.css(".item a").iter()
            .filter_map(|a| resp.follow_with(a.attr("href").unwrap_or(""), "detail"))
            .collect();
        (vec![], follows)
    })
    .on("detail", |resp| async move {
        let follows: Vec<_> = resp.css("article a").iter()
            .filter_map(|a| resp.follow_with(a.attr("href").unwrap_or(""), "content"))
            .collect();
        (vec![], follows)
    })
    .on("content", |resp| async move {
        (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
    })
    .build();
```

---

## 自检清单

**删除：**
- SessionManager 整模块（session.rs + session_test.rs）→ Task 1 ✅
- Spider trait 4 死方法（configure_sessions/session_for/concurrent_requests/rotate_ua）→ Task 1 ✅
- templates.rs 整文件 → Task 2 ✅
- parse_fn/async_parse_fn/ParseFn/AsyncParseFn 全删 → Task 3 ✅
- 7 个 dead_code warnings → Task 4 ✅

**新增：**
- SpiderBuilder::sitemap() 预设 → Task 2 ✅

**验证：**
- 编译零 warnings → Task 5 ✅
- 全量测试通过 → Task 5 ✅
- 无 parse_fn 残留 → Task 5 ✅
- 无 SessionManager 残留 → Task 5 ✅
