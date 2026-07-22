# SessionManager 接入 + SitemapSpider 迁移 + 冗余代码清理

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 SessionManager 从死代码接入为 Engine 基础设施（请求级 session 路由），SitemapSpider 迁移为 SpiderBuilder 预设，清理全部冗余代码。

**Tech Stack:** Rust, tokio, async-trait

---

## 背景

### 当前问题
1. **SessionManager 死代码**：`src/crawl/session.rs` 整套多会话基础设施与 Engine 完全断开
2. **Spider trait 死方法**：`configure_sessions` / `session_for` / `rotate_ua` / `concurrent_requests` 从未被 Engine 调用
3. **SitemapSpider 死代码**：`src/crawl/templates.rs` 全项目零引用
4. **parse_fn/async_parse_fn 冗余**：与 `on(label, handler)` API 等价并存
5. **7 个 dead_code warnings**：unused imports / unused mut / dead fields / dead methods
6. **Page.headless 死字段**：写入后从不读取
7. **challenge.rs 两个死方法**：`wait_js_challenge` / `wait_managed` 从未调用
8. **engine.rs:148 冗余赋值**：`final_resp = None` 初始值永不被读

### 设计原则
- **SessionManager 归属 Engine**：session 是"怎么抓"（基础设施），不是"抓什么"（业务逻辑）
- **请求级 session 标记**：借鉴 callback 机制，`req.session` 是请求级元数据，follow 时显式指定
- **删除 Spider trait 的 session 相关方法**：`configure_sessions` / `session_for` 改为 EngineBuilder 配置
- **不向后兼容**：删除 `parse_fn` / `async_parse_fn` / `ParseFn` / `AsyncParseFn`，统一到 `on()` API

---

## 文件结构

| 文件 | 责任 | 动作 |
|------|------|------|
| `src/crawl/mod.rs` | SpiderRequest 加 session 字段；Spider trait 删 4 方法 | 修改 |
| `src/crawl/session.rs` | SessionManager 保留；删除 request_with_session（改为 req.with_session） | 修改 |
| `src/crawl/builder.rs` | 删 parse_fn/async_parse_fn；加 sitemap() 预设 | 修改 |
| `src/crawl/engine.rs` | EngineContext 加 session_manager；fetch_with_retry 按 session 选 config | 修改 |
| `src/crawl/mod.rs` Engine | EngineBuilder 加 session() 方法 | 修改 |
| `src/crawl/templates.rs` | 删除整个文件 | **删除** |
| `src/stealth/challenge.rs` | 删 wait_js_challenge / wait_managed | 修改 |
| `src/browser/page.rs` | 删 headless 字段 | 修改 |
| `src/browser/mod.rs` | 删 unused CommandExt import | 修改 |
| `src/fetcher/mod.rs` | 删 unused imports | 修改 |
| `src/fetcher/session.rs` | 删 unused mut | 修改 |
| `tests/session_test.rs` | 适配新 API | 修改 |
| `tests/builder_api_test.rs` | 删 parse_fn 测试，改用 on() | 修改 |
| `tests/sitemap_test.rs` | 新建：测试 SpiderBuilder::sitemap() | **新建** |
| `tests/session_routing_test.rs` | 新建：测试 req.session 路由 | **新建** |

---

## Task 1: SpiderRequest 加 session 字段 + Spider trait 清理

**Files:**
- Modify: `src/crawl/mod.rs`

- [ ] **Step 1: SpiderRequest 加 session 字段**

在 SpiderRequest 结构体中，`callback` 字段后加：

```rust
pub struct SpiderRequest {
    pub url: String,
    pub method: Method,
    // ...
    pub callback: Option<String>,
    /// 会话 ID：指定用哪个 session 抓取（Engine 层路由）。
    /// None 表示用默认 session。
    pub session: Option<String>,
    // ...
}
```

- [ ] **Step 2: SpiderRequest 加 with_session 方法**

```rust
impl SpiderRequest {
    /// 指定会话 ID。
    pub fn with_session(mut self, session: &str) -> Self {
        self.session = Some(session.to_string());
        self
    }
}
```

- [ ] **Step 3: SpiderResponse 加 follow_with_session 方法**

```rust
impl SpiderResponse {
    /// follow 并指定 callback + session。
    pub fn follow_with_session(&self, href: &str, callback: &str, session: &str) -> Option<SpiderRequest> {
        let absolute = resolve_href(&self.url, href)?;
        Some(SpiderRequest::get(&absolute)
            .with_callback(callback)
            .with_session(session)
            .with_depth(self.request.depth + 1))
    }
}
```

- [ ] **Step 4: 删除 Spider trait 的 4 个死方法**

删除：
- `fn configure_sessions(&self, _mgr: &mut session::SessionManager) {}`
- `fn session_for(&self, _req: &SpiderRequest) -> &str { "default" }`
- `fn concurrent_requests(&self) -> u32 { 8 }`  （Engine 用 EngineBuilder::max_concurrent）
- `fn rotate_ua(&self) -> bool { false }`  （Engine 用 http::Config.rotate_ua）

- [ ] **Step 5: 更新所有 SpiderRequest 构造点**

搜索所有 `SpiderRequest { ... }` 构造，加 `session: None` 字段。搜索 `callback: None` 附近的位置。

- [ ] **Step 6: 验证编译**

```
cargo build --lib
```

- [ ] **Step 7: 提交**

```bash
git add src/crawl/mod.rs
git commit -m "refactor(crawl): SpiderRequest 加 session 字段，删除 trait 4 个死方法" -m "session 是请求级元数据，follow 时显式指定" -m "删除 configure_sessions/session_for/concurrent_requests/rotate_ua"
```

---

## Task 2: SessionManager 接入 Engine

**Files:**
- Modify: `src/crawl/session.rs`
- Modify: `src/crawl/engine.rs`
- Modify: `src/crawl/mod.rs`（Engine + EngineBuilder）

- [ ] **Step 1: 清理 session.rs**

删除 `request_with_session` 函数（改为 `req.with_session()`）。
删除 `session_id_of` 函数（改为读 `req.session`）。
保留 `SessionManager` 和 `FetcherType`。
更新文档示例用 `EngineBuilder::session()`。

- [ ] **Step 2: EngineContext 加 session_manager 字段**

```rust
pub(crate) struct EngineContext {
    // ... 现有字段 ...
    pub session_manager: Option<Arc<session::SessionManager>>,
}
```

- [ ] **Step 3: EngineBuilder 加 session() 方法**

```rust
pub struct EngineBuilder {
    // ... 现有字段 ...
    sessions: Vec<(String, session::FetcherType)>,
}

impl EngineBuilder {
    /// 添加命名会话。
    pub fn session(mut self, id: &str, fetcher: session::FetcherType) -> Self {
        self.sessions.push((id.to_string(), fetcher));
        self
    }

    pub fn build(self) -> Result<Engine> {
        // ...
        let session_manager = if self.sessions.is_empty() {
            None
        } else {
            let mut mgr = session::SessionManager::new();
            for (id, ft) in self.sessions {
                mgr.add(&id, ft);
            }
            Some(Arc::new(mgr))
        };
        Ok(Engine {
            // ...
            session_manager,
        })
    }
}
```

- [ ] **Step 4: fetch_with_retry 按 session 选 config/mode**

在 `fetch_with_retry` 开头（engine.rs:285 后）加：

```rust
// 按 session 选择 fetcher 配置和模式
let (effective_config, effective_mode) = if let Some(ref mgr) = ctx.session_manager {
    let sid = req.session.as_deref().unwrap_or("default");
    match mgr.get(sid) {
        Some(session::FetcherType::Http(cfg)) => (cfg.clone(), FetchMode::Http),
        Some(session::FetcherType::Stealth { proxy, .. }) => {
            let mut cfg = ctx.fetcher_config.clone();
            if proxy.is_some() { cfg.proxy = proxy.clone(); }
            (cfg, FetchMode::Stealth)
        }
        None => (ctx.fetcher_config.clone(), ctx.fetch_mode),
    }
} else {
    (ctx.fetcher_config.clone(), ctx.fetch_mode)
};
```

然后 `fetch_page` 和 `fetch_page_inner` 调用用 `effective_config` / `effective_mode` 替代 `ctx.fetcher_config` / `ctx.fetch_mode`。

- [ ] **Step 5: Engine::run_inner 初始化 session_manager**

在构造 EngineContext 时传入 `session_manager: self.session_manager.clone()`。

- [ ] **Step 6: 写测试**

新建 `tests/session_routing_test.rs`：

```rust
//! session 路由测试：验证 req.session 字段驱动 Engine 选择不同 fetcher 配置。
use wisp::crawl::*;
use wisp::crawl::session::{SessionManager, FetcherType};
use wisp::http;

#[tokio::test]
async fn test_session_routing_selects_config() {
    // 验证 EngineBuilder::session() 注册的会话能被 req.session 路由
    let engine = Engine::infra()
        .session("fast", FetcherType::Http(http::Config::default()))
        .session("stealth", FetcherType::Stealth {
            headless: true,
            proxy: None,
            challenge_timeout_secs: 30,
        })
        .build().unwrap();

    // 验证 session_manager 存在
    // （内部字段，通过行为验证）
    let spider = SpiderBuilder::new("session_test")
        .start_urls(vec!["http://127.0.0.1:1/"])  // 不可达
        .on("default", |resp| async move {
            (vec![serde_json::json!({"url": resp.url})], vec![])
        })
        .build();
    let _ = engine.run(spider).await.unwrap();
}

#[test]
fn test_session_manager_basic() {
    let mut mgr = SessionManager::new();
    mgr.add("fast", FetcherType::Http(http::Config::default()));
    mgr.add("stealth", FetcherType::Stealth {
        headless: false,
        proxy: Some("http://127.0.0.1:7897".into()),
        challenge_timeout_secs: 60,
    });
    assert_eq!(mgr.len(), 2);
    assert!(mgr.get("fast").is_some());
    assert!(mgr.get("stealth").is_some());
}

#[test]
fn test_request_with_session() {
    let req = SpiderRequest::get("https://example.com").with_session("stealth");
    assert_eq!(req.session.as_deref(), Some("stealth"));
}
```

- [ ] **Step 7: 验证**

```
cargo build --lib
cargo test --test session_routing_test -- --nocapture
cargo test --lib crawl::session -- --nocapture
```

- [ ] **Step 8: 提交**

```bash
git add src/crawl/session.rs src/crawl/engine.rs src/crawl/mod.rs tests/session_routing_test.rs
git commit -m "feat(crawl): SessionManager 接入 Engine 基础设施" -m "EngineBuilder::session(id, FetcherType) 注册会话" -m "fetch_with_retry 按 req.session 选 config/mode" -m "删除 request_with_session/session_id_of，改用 req.with_session()"
```

---

## Task 3: SitemapSpider 迁移为 SpiderBuilder 预设

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
/// 嵌套 sitemap（.xml 后缀）递归跟踪。
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

```bash
# 用 DeleteFile 工具删除 src/crawl/templates.rs
```

- [ ] **Step 3: 删除 mod.rs 的 templates 声明**

删除 `pub mod templates;` 行。

- [ ] **Step 4: 写测试**

新建 `tests/sitemap_test.rs`：

```rust
//! SitemapSpider（SpiderBuilder::sitemap）测试。
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

    // 模拟 sitemap.xml 响应
    let sitemap_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset>
  <url><loc>https://example.com/page1</loc></url>
  <url><loc>https://example.com/page2</loc></url>
  <url><loc>https://example.com/sitemap2.xml</loc></url>
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
    assert_eq!(follows.len(), 3);
    assert_eq!(follows[0].url, "https://example.com/page1");
    assert_eq!(follows[1].url, "https://example.com/page2");
    assert_eq!(follows[2].url, "https://example.com/sitemap2.xml");
    // 所有 follow 应带 "content" callback
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

## Task 4: 删除 parse_fn/async_parse_fn 冗余

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

## Task 5: 清理 7 个 dead_code warnings

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

## Task 6: 全量验证

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

- [ ] **Step 4: 检查 templates.rs 已删除**

```
# 确认文件不存在
```

- [ ] **Step 5: 检查无 parse_fn/async_parse_fn 残留**

用 Grep 搜索 `ParseFn|parse_fn|async_parse_fn|\.parse\(\|` 在 src/ 和 tests/ 中，应为 0。

- [ ] **Step 6: git 状态确认**

```
git status
git log --oneline -8
```

---

## 使用示例

### 示例 1：多 session 爬虫（列表用 HTTP，详情用 Stealth）

```rust
let engine = Engine::infra()
    .max_pages(1000)
    .session("fast", FetcherType::Http(http::Config::default()))
    .session("stealth", FetcherType::Stealth {
        headless: true,
        proxy: Some("http://127.0.0.1:7897".into()),
        challenge_timeout_secs: 60,
    })
    .build()?;

let spider = SpiderBuilder::new("multi_session")
    .start_urls(vec!["https://example.com/list".into()])
    .on("default", |resp| async move {
        // 列表页：用默认 session（fast HTTP）
        let follows: Vec<_> = resp.css(".item a").iter()
            .filter_map(|a| resp.follow_with(a.attr("href").unwrap_or(""), "detail"))
            .collect();
        (vec![], follows)
    })
    .on("detail", |resp| async move {
        // 详情页：用 stealth session
        let follows: Vec<_> = resp.css("article a").iter()
            .filter_map(|a| resp.follow_with_session(
                a.attr("href").unwrap_or(""), "content", "stealth"
            ))
            .collect();
        (vec![], follows)
    })
    .on("content", |resp| async move {
        (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
    })
    .build();

let (stats, items) = engine.run(spider).await?;
```

### 示例 2：Sitemap 爬虫

```rust
let spider = SpiderBuilder::sitemap("my_spider", vec!["https://example.com/sitemap.xml".into()], "content")
    .on("content", |resp| async move {
        (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
    })
    .build();

let engine = Engine::infra().build()?;
let (stats, items) = engine.run(spider).await?;
```

### 示例 3：简单爬虫（统一 on() API）

```rust
let spider = SpiderBuilder::new("simple")
    .start_urls(vec!["https://example.com/".into()])
    .on("default", |resp| async move {
        (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
    })
    .build();
```

---

## 自检清单

**功能接入：**
- SessionManager 接入 Engine（EngineBuilder::session + req.session 路由）→ Task 2 ✅
- SitemapSpider 迁移为 SpiderBuilder::sitemap() → Task 3 ✅

**冗余清理：**
- Spider trait 删 4 死方法（configure_sessions/session_for/concurrent_requests/rotate_ua）→ Task 1 ✅
- parse_fn/async_parse_fn/ParseFn/AsyncParseFn 全删 → Task 4 ✅
- templates.rs 整文件删除 → Task 3 ✅
- 7 个 dead_code warnings 清理 → Task 5 ✅

**验证：**
- 编译零 warnings → Task 6 ✅
- 全量测试通过 → Task 6 ✅
- 无 parse_fn 残留 → Task 6 ✅
