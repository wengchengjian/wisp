# wisp Crate 拆分与依赖环根治 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 把 wisp 从单 crate 拆成单向依赖的 workspace 多 crate，根治 `http <-> fetcher` 与 `fetcher -> crawl` 依赖环，同时保持 facade `wisp` 公共路径不变。

**Architecture:** 新建 `crates/` workspace，依赖方向严格为 `core <- storage/parser/proxy/http/browser <- stealth <- fetcher <- crawl <- mcp <- facade`。DTO（Request/Response/Method）、错误、配置、工具、编码下沉到 `wisp-core`；`Response` 的 HTML 解析方法移入 `wisp-parser` 的 `ResponseExt` trait，使 core 不依赖 parser；`sanitize_url` 移入 core utils，删除 fetcher 对 crawl 的反依赖。根 crate 保留为 facade，所有既有 `wisp::xxx` 路径通过模块 re-export 保持可用。

**Tech Stack:** Cargo workspace, Rust 2021, tokio, serde/serde_json, thiserror, wreq, woven-html

## Global Constraints

- 不考虑向后兼容：旧模块路径只在 facade 层保留，内部 crate 之间必须使用新 crate 路径。
- 依赖方向只允许：`wisp-core <- wisp-storage/wisp-parser/wisp-proxy/wisp-http/wisp-browser <- wisp-stealth <- wisp-fetcher <- wisp-crawl <- wisp-mcp <- wisp(facade)`。
- 禁止 `wisp-http` 依赖 `wisp-fetcher`；禁止 `wisp-fetcher` 依赖 `wisp-crawl`。
- 每个任务结束时必须 `cargo test --workspace` 或 `cargo test --lib` 通过，且该 commit 独立可编译。
- 代码保持 snake_case；提交信息用中文。
- 当前分支 `codex/multi-spider-refactor` 有未提交改动；Task 0 必须先确认基线。
- 默认 ASCII；中文只出现在注释和提交信息中。

---

## 文件结构（目标态）

```text
Cargo.toml                       workspace + facade package（bin/example/test/bench 留在根）
src/lib.rs                       facade：pub use wisp_xxx as xxx + 顶层 re-export
src/bin/wisp.rs                  CLI（不变）
crates/core/                     Request/Response/Method/WispError/config/text/utils/encoding
crates/storage/                  Store + Memory/File/Sqlite + CachedResponse/ElementSnapshotRow
crates/parser/                   Node/NodeList/adaptive/difflib + ResponseExt
crates/proxy/                    ProxyPool/RotationStrategy + config_file
crates/http/                     wreq Client/ClientBuilder/Config/DomainBlocker/UaRotator/ParsedProxy
crates/browser/                  CDP browser 全部模块
crates/stealth/                  challenge/human/turnstile
crates/fetcher/                  Fetcher/FetchClient/FetchMode
crates/crawl/                    Engine/Spider/middleware/scheduler/runtime/observability
crates/mcp/                      MCP server
tests/ examples/ benches/        留在根，只依赖 wisp facade
```

---

## Task 0: 基线确认与 workspace 脚手架

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/core/Cargo.toml`, `crates/core/src/lib.rs`

**Interfaces:**
- Produces: 根 workspace（当前仅含 facade + `crates/core`）；后续任务逐个把新 crate 加入 `[workspace].members`。

- [x] **Step 1: 确认并提交当前基线**

若 `git status --short` 有输出，先向用户确认这些是本次会话的既有改动，然后：

```bash
git add -A
git commit -m "wip: multi-spider refactor baseline before crate split"
```

Expected: `git status --short` 为空。

- [x] **Step 2: 创建拆分分支**

```bash
git checkout -b codex/crate-split
```

Expected: 当前分支为 `codex/crate-split`。

- [x] **Step 3: 修改根 Cargo.toml 为 workspace + facade**

用 `apply_patch` 把根 `Cargo.toml` 开头改为：

```toml
[workspace]
members = ["crates/core"]
resolver = "2"

[package]
name = "wisp"
version = "0.1.0"
edition = "2021"
description = "Lightweight undetected browser automation for Rust - built for scraping"
license = "Apache-2.0"

[features]
default = []
# 启用 SQLite 存储后端（默认禁用，使用 FileStore）
sqlite = ["dep:rusqlite"]
```

根 `[dependencies]` 暂时保留原样（后续任务再逐步删减），保证此步编译通过。

- [x] **Step 4: 创建 wisp-core 骨架**

`crates/core/Cargo.toml`：

```toml
[package]
name = "wisp-core"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
url = "2"
regex = "1"
tracing = "0.1"
rand = "0.10"
encoding_rs = "0.8"
```

`crates/core/src/lib.rs`：

```rust
//! wisp 共享基础层：DTO、错误、配置、文本/URL 工具、编码。

pub mod config;
pub mod encoding;
pub mod error;
pub mod text;
pub mod types;
pub mod utils;

pub use types::{Method, Request, Response};
```

- [x] **Step 5: 验证骨架可编译**

```bash
cargo check
```

Expected: 通过（core 骨架暂时只有空模块声明，但 `src/error.rs` 等尚未移动，根 crate 仍完整）。

- [x] **Step 6: 提交**

```bash
git add Cargo.toml crates/core
git commit -m "chore(workspace): scaffold workspace and wisp-core skeleton"
```

---

## Task 1: wisp-core 内容下沉（DTO/错误/配置/工具/编码 + ResponseExt 解耦）

**Files:**
- Move: `src/error.rs` -> `crates/core/src/error.rs`
- Move: `src/config.rs` -> `crates/core/src/config.rs`
- Move: `src/text.rs` -> `crates/core/src/text.rs`
- Move: `src/utils/mod.rs`, `src/utils/http.rs`, `src/utils/port.rs`, `src/utils/random.rs`, `src/utils/ssrf.rs`, `src/utils/url.rs` -> `crates/core/src/utils/`
- Move: `src/http/encoding.rs` -> `crates/core/src/encoding.rs`
- Create: `crates/core/src/types.rs`（由 `src/fetcher/response.rs` 精简而来）
- Modify: `crates/core/src/utils/mod.rs`（新增 `sanitize_url`）
- Modify: `src/fetcher/mod.rs`（re-export core 类型）
- Delete: `src/fetcher/response.rs`
- Create: `src/parser/response_ext.rs`（`ResponseExt` trait）
- Modify: `src/parser/mod.rs`（导出 `ResponseExt`）
- Modify: `src/crawl/page.rs`, `src/crawl/builder.rs`, `src/crawl/engine.rs`, `src/fetcher/client.rs` 及所有调用 `Response::parse/css/select_one/find_by_text` 的 src/tests/examples
- Test: `tests/page_api_test.rs`、`src/parser/response_ext.rs` 内新增 trait 测试

**Interfaces:**
- Consumes: Task 0 的 `wisp-core` 骨架。
- Produces: `wisp_core::{Request, Response, Method}`（无 parser 方法）、`wisp_core::utils::sanitize_url`、`wisp_parser::ResponseExt`（本任务暂在根 parser 内实现，Task 3 再整体搬入 `wisp-parser`）。

- [x] **Step 1: 移动纯机械文件**

```bash
git mv src/error.rs crates/core/src/error.rs
git mv src/config.rs crates/core/src/config.rs
git mv src/text.rs crates/core/src/text.rs
git mv src/utils crates/core/src/utils
git mv src/http/encoding.rs crates/core/src/encoding.rs
```

Expected: 文件存在于 `crates/core/src/` 对应位置。

- [x] **Step 2: 修正 core 内部引用**

用 `apply_patch` 替换（文件内容其余不变）：
- `crates/core/src/utils/ssrf.rs`：`use crate::error::{Result, WispError, McpError};` -> `use crate::error::{McpError, Result, WispError};`
- `crates/core/src/utils/mod.rs`：新增导出

```rust
pub use url::{resolve_href, url_to_filename};
```

改为：

```rust
pub use url::{resolve_href, sanitize_url, url_to_filename};
```

并把原 `src/crawl/engine.rs` 中的 `sanitize_url` 函数整体移入 `crates/core/src/utils/url.rs`，签名不变：

```rust
/// ND-007-SEC：脱敏 URL 中的凭据（user:pass@host）。
///
/// 错误信息和日志中不应包含 URL 凭据。此函数将 `http://user:pass@host/path`
/// 转换为 `http://***:***@host/path`，避免凭据泄露。
pub fn sanitize_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => {
            if u.username().is_empty() {
                return url.to_string();
            }
            let mut s = url.to_string();
            if let Some(at_pos) = s.find('@') {
                if let Some(scheme_end) = s.find("://") {
                    let creds_start = scheme_end + 3;
                    if at_pos > creds_start {
                        let creds = &s[creds_start..at_pos];
                        s = s.replace(creds, "***:***");
                    }
                }
            }
            s
        }
        Err(_) => url.to_string(),
    }
}
```

`crates/core/src/utils/url.rs` 末尾追加对应测试（`sanitize_url_no_credentials`、`sanitize_url_redacts_credentials`、`sanitize_url_invalid_url_passthrough`、`sanitize_url_only_username`），内容从原 `src/crawl/engine.rs` 测试原样复制。

`crates/core/src/encoding.rs` 中 `decode_borrowed` 从 `pub(crate)` 改为 `pub`（Task 3 的 `ResponseExt` 需要从外部 crate 调用）：

```rust
/// 解码为可能借用原始 body 的字符串，UTF-8 页面可避免一次 String 复制。
pub fn decode_borrowed<'a>(body: &'a [u8], content_type: &str) -> Cow<'a, str> {
```

- [x] **Step 3: 从 response.rs 生成 core types.rs**

```bash
git mv src/fetcher/response.rs crates/core/src/types.rs
```

然后用 `apply_patch` 修改 `crates/core/src/types.rs`：

1. 删除 import：`use crate::error::{WispError, Result, ParseError};`、`use crate::parser::{Node, NodeList};`
2. 改为：`use crate::encoding;`、`use crate::error::{ParseError, Result, WispError};`、`use crate::utils::resolve_href;`
3. 删除 `Response` 中的 `parsed: AtomicBool` 字段、`parse()`、`css()`、`find_by_text()`、`select_one()` 四个方法。
4. 删除 `Response` 的手动 `Clone`/`Debug` impl，改为 derive：

```rust
#[derive(Debug, Clone)]
pub struct Response {
    /// HTTP 状态码
    pub status: u16,
    /// 最终 URL（重定向后）
    pub url: String,
    /// 响应头
    pub headers: HashMap<String, String>,
    /// 响应体原始字节
    pub body: Vec<u8>,
    /// 浏览器模式下的页面标题
    pub title: Option<String>,
    /// 浏览器模式下的 cookies
    pub cookies: Vec<String>,
    /// 发起此响应的请求（用于 follow()）
    pub request: Request,
    /// Content-Type 头（用于编码检测）
    pub content_type: String,
    /// 是否来自缓存（缓存命中不算 pages_crawled）。
    #[doc(hidden)]
    pub from_cache: bool,
}
```

5. `Response::text()` 改为 `Ok(crate::encoding::decode(&self.body, &self.content_type))`。
6. `text.rs`、`utils/*`、`config.rs`、`error.rs`、`encoding.rs` 内部的 `crate::` 相对路径在 core 内继续有效（模块树一致），无需再改。
7. 删除 `use std::sync::atomic::{AtomicBool, Ordering};`（`parsed` 字段已删除）。

- [x] **Step 4: 建立根模块 shim，保持根 crate 内部编译**

`src/error.rs` 整文件替换为：

```rust
//! 分类错误体系（re-export 自 wisp-core）。
pub use wisp_core::error::*;
```

`src/config.rs` 整文件替换为：

```rust
//! 浏览器启动选项和代理配置（re-export 自 wisp-core）。
pub use wisp_core::config::*;
```

`src/text.rs` 整文件替换为：

```rust
//! Text and attribute processing utilities（re-export 自 wisp-core）。
pub use wisp_core::text::*;
```

`src/utils/mod.rs` 整文件替换为：

```rust
//! 内部辅助工具（re-export 自 wisp-core）。
pub use wisp_core::utils::*;
```

`src/http/encoding.rs` 整文件替换为：

```rust
//! Character encoding detection and decoding（re-export 自 wisp-core）。
pub use wisp_core::encoding::*;
```

`src/fetcher/mod.rs` 顶部增加并保留（其它内容不动）：

```rust
// Request/Response 已下沉到 wisp-core，为兼容内部路径继续 re-export。
pub use wisp_core::{Method, Request, Response};
```

同时删除原 `src/fetcher/mod.rs` 中的 `pub use response::{Method, Request, Response};`。

`src/fetcher/client.rs` 的导入：

```rust
use super::response::{Request, Response};
```

改为：

```rust
use wisp_core::{Request, Response};
```

根 `Cargo.toml` 的 `[dependencies]` 增加：

```toml
wisp-core = { path = "crates/core" }
```

- [x] **Step 5: 新增 ResponseExt trait 并替换解析调用点**

创建 `src/parser/response_ext.rs`：

```rust
//! `Response` 的 HTML 解析扩展：core DTO 不依赖 parser，解析能力由本 trait 提供。

use crate::parser::{Node, NodeList};
use wisp_core::Response;

/// 在 `Response` 上解析 HTML 的扩展接口。
pub trait ResponseExt {
    /// 解析 HTML 为文档节点。
    fn parse(&self) -> Node;
    /// CSS 选择器查询。
    fn css(&self, selector: &str) -> NodeList;
    /// CSS 选择器查询第一个匹配元素。
    fn select_one(&self, selector: &str) -> Option<Node>;
    /// 按文本内容查找元素。
    fn find_by_text(&self, text: &str, tag: Option<&str>, exact: bool) -> NodeList;
}

impl ResponseExt for Response {
    fn parse(&self) -> Node {
        let text = wisp_core::encoding::decode_borrowed(&self.body, &self.content_type);
        Node::from_html_owned(text.into_owned())
    }

    fn css(&self, selector: &str) -> NodeList {
        self.parse().select(selector)
    }

    fn select_one(&self, selector: &str) -> Option<Node> {
        self.parse().select_one(selector)
    }

    fn find_by_text(&self, text: &str, tag: Option<&str>, exact: bool) -> NodeList {
        self.parse().find_by_text(text, tag, exact)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use wisp_core::{Request, Response};

    fn make_response(html: &str) -> Response {
        Response::from_http(
            200,
            "https://example.com/page".to_string(),
            HashMap::new(),
            html.as_bytes().to_vec(),
            "text/html; charset=utf-8".to_string(),
            Request::get("https://example.com/page"),
        )
    }

    #[test]
    fn response_ext_css_works() {
        let resp = make_response(r#"<div class="item">A</div><div class="item">B</div>"#);
        assert_eq!(resp.css(".item").len(), 2);
    }

    #[test]
    fn response_ext_parse_then_query() {
        let resp = make_response("<h1>Title</h1>");
        let doc = resp.parse();
        assert_eq!(doc.select_one("h1").map(|n| n.text()), Some("Title".to_string()));
    }

    #[test]
    fn response_ext_select_one() {
        let resp = make_response(r#"<p id="main">Content</p>"#);
        assert_eq!(resp.select_one("#main").map(|n| n.text()), Some("Content".to_string()));
    }

    #[test]
    fn response_ext_find_by_text() {
        let resp = make_response(r#"<div>Apple</div><div>Banana</div>"#);
        assert_eq!(resp.find_by_text("Apple", Some("div"), true).len(), 1);
    }
}
```

`src/parser/mod.rs` 增加：

```rust
mod response_ext;
pub use response_ext::ResponseExt;
```

删除 `crates/core/src/types.rs` 中已经消失的旧测试（`test_response_css_works`、`test_response_parse_twice_panics`、`test_response_css_then_select_one_panics`、`test_response_clone_resets_parsed_flag`），保留 text/json/follow 相关测试。

- [x] **Step 6: 更新内部调用点**

运行：

```powershell
rg -n "resp\.(parse|css|select_one|find_by_text)\(|page\.resp\(\)\.(parse|css|select_one|find_by_text)\(" src tests examples
```

对每一个命中的文件：
- src 内文件加 `use crate::parser::ResponseExt;`（`page.rs`、`builder.rs`、`crawl/mod.rs` 测试模块等）。
- tests/examples 文件加 `use wisp::parser::ResponseExt;`。

`src/fetcher/client.rs:316` 的 `crate::crawl::engine::sanitize_url(&req.url)` 改为 `crate::utils::sanitize_url(&req.url)`。

`src/crawl/engine.rs` 删除本地 `sanitize_url` 函数定义，文件内所有 `sanitize_url(...)` 调用改为 `crate::utils::sanitize_url(...)`；`runner.rs` 中的 `engine::sanitize_url` 同样改为 `crate::utils::sanitize_url`。

`src/lib.rs` 的 Quick Start doctest 把 `use wisp::Fetcher;` 替换为：

```rust
use wisp::{Fetcher, parser::ResponseExt};
```

（否则 doctest 中 `page.css(...)` 无法解析。）

- [x] **Step 7: 编译并跑测试**

```bash
cargo test --lib
```

Expected: 全部通过（原 278 passed / 8 ignored；`Response` 解析测试迁移到 `parser::response_ext`）。

- [x] **Step 8: 修复剩余引用直到 `cargo test --workspace` 通过**

```bash
cargo test --workspace
```

Expected: 根 workspace 所有 lib + integration tests 通过。

- [x] **Step 9: 提交**

```bash
git add -A
git commit -m "refactor(core): extract wisp-core types/error/config/utils/encoding"
```

---

## Task 2: wisp-storage 拆分

**Files:**
- Move: `src/storage/mod.rs`, `src/storage/memory.rs`, `src/storage/file.rs`, `src/storage/migrations.rs` -> `crates/storage/src/`
- Move: `src/storage/sqlite.rs` -> `crates/storage/src/sqlite.rs`
- Create: `crates/storage/Cargo.toml`, `crates/storage/src/lib.rs`
- Replace: `src/storage/mod.rs`（shim）
- Modify: `Cargo.toml`（workspace members + facade 依赖）

**Interfaces:**
- Consumes: `wisp_core::error`。
- Produces: `wisp_storage::{Store, MemoryStore, FileStore, CachedResponse, ElementSnapshotRow}`；feature `sqlite`。

- [x] **Step 1: 创建 crate 并移动文件**

```bash
New-Item -ItemType Directory -Force crates/storage/src | Out-Null
git mv src/storage/mod.rs crates/storage/src/lib.rs
git mv src/storage/memory.rs crates/storage/src/memory.rs
git mv src/storage/file.rs crates/storage/src/file.rs
git mv src/storage/migrations.rs crates/storage/src/migrations.rs
git mv src/storage/sqlite.rs crates/storage/src/sqlite.rs
```

`crates/storage/Cargo.toml`：

```toml
[package]
name = "wisp-storage"
version = "0.1.0"
edition = "2021"

[features]
default = []
sqlite = ["dep:rusqlite"]

[dependencies]
wisp-core = { path = "../core" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bincode = "1.3.3"
chrono = { version = "0.4", features = ["serde"] }
moka = { version = "0.12", features = ["future", "sync"] }
parking_lot = "0.12"
rusqlite = { version = "0.39", features = ["bundled"], optional = true }
```

- [x] **Step 2: 修正 crate 内部引用**

`crates/storage/src/lib.rs`、`memory.rs`、`file.rs`、`sqlite.rs`、`migrations.rs` 中所有 `crate::error::` -> `wisp_core::error::`，并在 `lib.rs` 顶部加 `use wisp_core::error::{Result, WispError, StorageError};`。

`src/storage/mod.rs` 整文件替换为：

```rust
//! 统一存储层（re-export 自 wisp-storage）。
pub use wisp_storage::*;
```

根 `Cargo.toml`：
- `[workspace].members` 增加 `"crates/storage"`。
- `[features] sqlite = ["wisp-storage/sqlite"]`（Task 0 只保留了 `dep:rusqlite` 占位，现在替换）。
- `[dependencies]` 增加 `wisp-storage = { path = "crates/storage" }`。

- [x] **Step 3: 编译并跑测试**

```bash
cargo test --workspace
```

Expected: 通过。

- [x] **Step 4: 提交**

```bash
git add -A
git commit -m "refactor(storage): extract wisp-storage crate"
```

---

## Task 3: wisp-parser 拆分（含 ResponseExt）

**Files:**
- Move: `src/parser/mod.rs`, `src/parser/document.rs`, `src/parser/adaptive.rs`, `src/parser/difflib.rs`, `src/parser/generate.rs`, `src/parser/response_ext.rs` -> `crates/parser/src/`
- Create: `crates/parser/Cargo.toml`
- Replace: `src/parser/mod.rs`（shim）
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `wisp-core`（`Response`、`encoding`）、`wisp-storage`（`Store`、`ElementSnapshotRow`）。
- Produces: `wisp_parser::{Node, NodeList, ResponseExt}` 及 `css_adaptive` 等。

- [x] **Step 1: 创建 crate 并移动文件**

```bash
New-Item -ItemType Directory -Force crates/parser/src | Out-Null
git mv src/parser/mod.rs crates/parser/src/lib.rs
git mv src/parser/document.rs crates/parser/src/document.rs
git mv src/parser/adaptive.rs crates/parser/src/adaptive.rs
git mv src/parser/difflib.rs crates/parser/src/difflib.rs
git mv src/parser/generate.rs crates/parser/src/generate.rs
git mv src/parser/response_ext.rs crates/parser/src/response_ext.rs
```

`crates/parser/Cargo.toml`：

```toml
[package]
name = "wisp-parser"
version = "0.1.0"
edition = "2021"

[dependencies]
wisp-core = { path = "../core" }
wisp-storage = { path = "../storage" }
woven-html = "0.1"
serde_json = "1"
regex = "1"
```

- [x] **Step 2: 修正 crate 内部引用**

- `crates/parser/src/adaptive.rs`：`use crate::storage::{ElementSnapshotRow, Store};` -> `use wisp_storage::{ElementSnapshotRow, Store};`
- `crates/parser/src/response_ext.rs`：`use crate::parser::{Node, NodeList};` -> `use crate::{Node, NodeList};`
- `crates/parser/src/lib.rs` 顶部模块声明保持：

```rust
pub mod adaptive;
pub mod difflib;
pub mod document;
pub mod generate;

mod response_ext;
pub use response_ext::ResponseExt;
```

- [x] **Step 3: 建立根 shim**

`src/parser/mod.rs` 整文件替换为：

```rust
//! HTML parsing（re-export 自 wisp-parser）。
pub use wisp_parser::*;
```

根 `Cargo.toml`：`members` 增加 `"crates/parser"`，`[dependencies]` 增加 `wisp-parser = { path = "crates/parser" }`。

- [x] **Step 4: 编译并跑测试**

```bash
cargo test --workspace
```

Expected: 通过。

- [x] **Step 5: 提交**

```bash
git add -A
git commit -m "refactor(parser): extract wisp-parser crate with ResponseExt"
```

---

## Task 4: wisp-proxy 拆分（含 config_file）

**Files:**
- Move: `src/proxy.rs` -> `crates/proxy/src/lib.rs`
- Move: `src/config_file.rs` -> `crates/proxy/src/config_file.rs`
- Create: `crates/proxy/Cargo.toml`
- Replace: `src/proxy.rs`, `src/config_file.rs`（shim）
- Modify: `Cargo.toml`

**Interfaces:**
- Produces: `wisp_proxy::{ProxyPool, RotationStrategy}`、`wisp_proxy::config_file::{WispConfig, EngineConfig, ProxyConfig, HttpConfig, StealthConfig}`。

- [x] **Step 1: 创建 crate 并移动文件**

```bash
New-Item -ItemType Directory -Force crates/proxy/src | Out-Null
git mv src/proxy.rs crates/proxy/src/lib.rs
git mv src/config_file.rs crates/proxy/src/config_file.rs
```

`crates/proxy/Cargo.toml`：

```toml
[package]
name = "wisp-proxy"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
toml = "1"
rand = "0.10"
```

- [x] **Step 2: 修正引用**

`crates/proxy/src/config_file.rs`：`use crate::proxy::RotationStrategy;` -> `use crate::RotationStrategy;`。

`src/proxy.rs` 整文件替换为：

```rust
//! Proxy pool management（re-export 自 wisp-proxy）。
pub use wisp_proxy::*;
```

`src/config_file.rs` 整文件替换为：

```rust
//! 外部配置文件支持（re-export 自 wisp-proxy）。
pub use wisp_proxy::config_file::*;
```

根 `Cargo.toml`：`members` 增加 `"crates/proxy"`，`[dependencies]` 增加 `wisp-proxy = { path = "crates/proxy" }`。

- [x] **Step 3: 编译并跑测试**

```bash
cargo test --workspace
```

Expected: 通过。

- [x] **Step 4: 提交**

```bash
git add -A
git commit -m "refactor(proxy): extract wisp-proxy crate"
```

---

## Task 5: wisp-http 拆分（移除对 fetcher 的依赖）

**Files:**
- Move: `src/http/mod.rs`, `src/http/block.rs`, `src/http/proxy.rs`, `src/http/ua.rs` -> `crates/http/src/`
- Create: `crates/http/Cargo.toml`
- Replace: `src/http/mod.rs`（shim；`encoding.rs` 已在前序任务删除）
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `wisp_core::{Method, Request, Response}`、`wisp_core::error`。
- Produces: `wisp_http::{Client, ClientBuilder, Config, DomainBlocker, UaRotator}`。

- [x] **Step 1: 创建 crate 并移动文件**

```bash
New-Item -ItemType Directory -Force crates/http/src | Out-Null
git mv src/http/mod.rs crates/http/src/lib.rs
git mv src/http/block.rs crates/http/src/block.rs
git mv src/http/proxy.rs crates/http/src/proxy.rs
git mv src/http/ua.rs crates/http/src/ua.rs
```

`crates/http/Cargo.toml`：

```toml
[package]
name = "wisp-http"
version = "0.1.0"
edition = "2021"

[dependencies]
wisp-core = { path = "../core" }
serde_json = "1"
url = "2"
futures = "0.3"
tracing = "0.1"
rand = "0.10"
wreq = { version = "=6.0.0-rc.29", features = ["cookies", "stream"] }
wreq-util = "=3.0.0-rc.14"
```

- [x] **Step 2: 替换 DTO 引用**

`crates/http/src/lib.rs`：

```rust
use crate::error::{Result, WispError, NetworkError, ParseError};
use wisp_core::{Method as FetchMethod, Request as FetchRequest, Response as FetchResponse};
```

`crates/http/src/lib.rs` 中 `use crate::error::` -> `use wisp_core::error::`（其余错误引用同步替换）。

`crates/http/src/lib.rs` 的模块声明删除 `encoding`（该模块已下沉到 `wisp-core`）：

```rust
pub mod block;
pub mod proxy;
pub mod ua;
```

- [x] **Step 3: 建立根 shim**

`src/http/mod.rs` 整文件替换为：

```rust
//! HTTP client（re-export 自 wisp-http）。
pub use wisp_http::*;
```

根 `Cargo.toml`：`members` 增加 `"crates/http"`，`[dependencies]` 增加 `wisp-http = { path = "crates/http" }`。

- [x] **Step 4: 验证依赖方向**

```powershell
rg -n "wisp_fetcher|wisp-crawl" crates/http
```

Expected: 无输出。

- [x] **Step 5: 编译并跑测试**

```bash
cargo test --workspace
```

Expected: 通过。

- [x] **Step 6: 提交**

```bash
git add -A
git commit -m "refactor(http): extract wisp-http crate and decouple from fetcher"
```

---

## Task 6: wisp-browser 拆分

**Files:**
- Move: `src/browser/mod.rs`, `cdp.rs`, `element.rs`, `installer.rs`, `launch.rs`, `page.rs`, `patches.rs`, `pool.rs` -> `crates/browser/src/`
- Create: `crates/browser/Cargo.toml`
- Replace: `src/browser/mod.rs`（shim）
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `wisp_core::config::{LaunchOptions, ProxyConfig}`、`wisp_core::error`。
- Produces: `wisp_browser::{Browser, Page, CdpSession, BrowserPool}`。

- [x] **Step 1: 创建 crate 并移动文件**

```bash
New-Item -ItemType Directory -Force crates/browser/src | Out-Null
git mv src/browser/mod.rs crates/browser/src/lib.rs
git mv src/browser/cdp.rs crates/browser/src/cdp.rs
git mv src/browser/element.rs crates/browser/src/element.rs
git mv src/browser/installer.rs crates/browser/src/installer.rs
git mv src/browser/launch.rs crates/browser/src/launch.rs
git mv src/browser/page.rs crates/browser/src/page.rs
git mv src/browser/patches.rs crates/browser/src/patches.rs
git mv src/browser/pool.rs crates/browser/src/pool.rs
```

`crates/browser/Cargo.toml`：

```toml
[package]
name = "wisp-browser"
version = "0.1.0"
edition = "2021"

[dependencies]
wisp-core = { path = "../core" }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
tokio-tungstenite = { version = "0.30", features = ["rustls-tls-webpki-roots"] }
tungstenite = "0.30"
which = "7"
zip = "2"
tempfile = "3"
base64 = "0.22"
rand = "0.10"
futures = "0.3"
regex = "1"
url = "2"
tracing = "0.1"
```

- [x] **Step 2: 修正 crate 内部引用**

`crates/browser/src/*.rs` 中：
- `use crate::config::LaunchOptions;` -> `use wisp_core::config::LaunchOptions;`
- `use crate::error::{...};` -> `use wisp_core::error::{...};`
- `crate::browser::` 内部相对路径不变（模块树一致）。

- [x] **Step 3: 建立根 shim**

`src/browser/mod.rs` 整文件替换为：

```rust
//! Browser process management（re-export 自 wisp-browser）。
pub use wisp_browser::*;
```

根 `Cargo.toml`：`members` 增加 `"crates/browser"`，`[dependencies]` 增加 `wisp-browser = { path = "crates/browser" }`。

- [x] **Step 4: 编译并跑测试**

```bash
cargo test --workspace
```

Expected: 通过（浏览器相关 integration tests 可能需要真实 Chrome，允许标 ignore 或本地验证）。

- [x] **Step 5: 提交**

```bash
git add -A
git commit -m "refactor(browser): extract wisp-browser crate"
```

---

## Task 7: wisp-stealth 拆分

**Files:**
- Move: `src/stealth/mod.rs`, `challenge.rs`, `human.rs`, `turnstile.rs` -> `crates/stealth/src/`
- Create: `crates/stealth/Cargo.toml`
- Replace: `src/stealth/mod.rs`（shim）
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `wisp-core`、`wisp-browser::page::Page`。
- Produces: `wisp_stealth::{ChallengeSolver, ChallengeType, is_cloudflare_page, HumanBehavior, TurnstileConfig}`。

- [x] **Step 1: 创建 crate 并移动文件**

```bash
New-Item -ItemType Directory -Force crates/stealth/src | Out-Null
git mv src/stealth/mod.rs crates/stealth/src/lib.rs
git mv src/stealth/challenge.rs crates/stealth/src/challenge.rs
git mv src/stealth/human.rs crates/stealth/src/human.rs
git mv src/stealth/turnstile.rs crates/stealth/src/turnstile.rs
```

`crates/stealth/Cargo.toml`：

```toml
[package]
name = "wisp-stealth"
version = "0.1.0"
edition = "2021"

[dependencies]
wisp-core = { path = "../core" }
wisp-browser = { path = "../browser" }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
rand = "0.10"
tracing = "0.1"
chrono = { version = "0.4", features = ["serde"] }
```

- [x] **Step 2: 修正 crate 内部引用**

- `crates/stealth/src/challenge.rs`、`human.rs`、`turnstile.rs`：`use crate::browser::page::Page;` -> `use wisp_browser::page::Page;`
- `challenge.rs` 测试中的 `use crate::browser::Browser;` -> `use wisp_browser::Browser;`
- `use crate::config::LaunchOptions;` -> `use wisp_core::config::LaunchOptions;`
- `use crate::error::{...};` -> `use wisp_core::error::{...};`

- [x] **Step 3: 建立根 shim**

`src/stealth/mod.rs` 整文件替换为：

```rust
//! Stealth module（re-export 自 wisp-stealth）。
pub use wisp_stealth::*;
```

根 `Cargo.toml`：`members` 增加 `"crates/stealth"`，`[dependencies]` 增加 `wisp-stealth = { path = "crates/stealth" }`。

- [x] **Step 4: 编译并跑测试**

```bash
cargo test --workspace
```

Expected: 通过。

- [x] **Step 5: 提交**

```bash
git add -A
git commit -m "refactor(stealth): extract wisp-stealth crate"
```

---

## Task 8: wisp-fetcher 拆分（删除对 crawl 的反依赖）

**Files:**
- Move: `src/fetcher/mod.rs`, `src/fetcher/client.rs` -> `crates/fetcher/src/`
- Create: `crates/fetcher/Cargo.toml`
- Replace: `src/fetcher/mod.rs`（shim）
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `wisp_core::{Request, Response, Method}`、`wisp_http::Client`、`wisp_browser::BrowserPool`、`wisp_stealth::{ChallengeSolver, HumanBehavior}`。
- Produces: `wisp_fetcher::{Fetcher, FetcherBuilder, FetchMode, FetchClient, FetchClientConfig}`；并 re-export `pub use wisp_core::{Method, Request, Response};`。

- [x] **Step 1: 创建 crate 并移动文件**

```bash
New-Item -ItemType Directory -Force crates/fetcher/src | Out-Null
git mv src/fetcher/mod.rs crates/fetcher/src/lib.rs
git mv src/fetcher/client.rs crates/fetcher/src/client.rs
```

`crates/fetcher/Cargo.toml`：

```toml
[package]
name = "wisp-fetcher"
version = "0.1.0"
edition = "2021"

[dependencies]
wisp-core = { path = "../core" }
wisp-http = { path = "../http" }
wisp-browser = { path = "../browser" }
wisp-stealth = { path = "../stealth" }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
url = "2"
tracing = "0.1"
moka = { version = "0.12", features = ["future", "sync"] }
chrono = { version = "0.4", features = ["serde"] }
wreq-util = "=3.0.0-rc.14"
```

- [x] **Step 2: 修正 crate 内部引用**

`crates/fetcher/src/lib.rs` 增加：

```rust
pub use wisp_core::{Method, Request, Response};
```

`crates/fetcher/src/client.rs`：
- `use crate::browser::BrowserPool;` -> `use wisp_browser::BrowserPool;`
- `use crate::config::LaunchOptions;` -> `use wisp_core::config::LaunchOptions;`
- `use crate::error::{...};` -> `use wisp_core::error::{...};`
- `use crate::http::{block::DomainBlocker, Client};` -> `use wisp_http::{block::DomainBlocker, Client};`
- `use crate::stealth::challenge::ChallengeSolver;` / `use crate::stealth::human::HumanBehavior;` -> `use wisp_stealth::challenge::ChallengeSolver;` / `use wisp_stealth::human::HumanBehavior;`
- `use crate::http::Client;` -> `use wisp_http::Client;`
- `do_browser_work_inner` 签名中的 `crate::browser::page::Page` -> `wisp_browser::page::Page`
- `crate::crawl::engine::sanitize_url` 已在本计划 Task 1 改为 `crate::utils::sanitize_url`，此处改为 `use wisp_core::utils::sanitize_url;`

`crates/fetcher/src/lib.rs` 中 `use crate::error::Result;` -> `use wisp_core::error::Result;`。

- [x] **Step 3: 建立根 shim**

`src/fetcher/mod.rs` 整文件替换为：

```rust
//! Unified Fetcher API（re-export 自 wisp-fetcher）。
pub use wisp_fetcher::*;
```

根 `Cargo.toml`：`members` 增加 `"crates/fetcher"`，`[dependencies]` 增加 `wisp-fetcher = { path = "crates/fetcher" }`。

- [x] **Step 4: 验证依赖方向**

```powershell
rg -n "wisp_crawl|wisp-crawl" crates/fetcher
```

Expected: 无输出。

- [x] **Step 5: 编译并跑测试**

```bash
cargo test --workspace
```

Expected: 通过。

- [x] **Step 6: 提交**

```bash
git add -A
git commit -m "refactor(fetcher): extract wisp-fetcher crate"
```

---

## Task 9: wisp-crawl 拆分

**Files:**
- Move: `src/crawl/` 全部子模块 -> `crates/crawl/src/`
- Create: `crates/crawl/Cargo.toml`
- Replace: `src/crawl/mod.rs`（shim）
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `wisp_core`、`wisp_fetcher`、`wisp_http`、`wisp_parser`、`wisp_storage`、`wisp_proxy`。
- Produces: `wisp_crawl::{Spider, Engine, SpiderBuilder, ClosureSpider, Page, CrawlEvent, CrawlStream, Items, JsonlWriter, CrawlStats}` 及 `middleware/scheduling/runtime/observability` 子模块。

- [x] **Step 1: 创建 crate 并移动文件**

```bash
New-Item -ItemType Directory -Force crates/crawl/src | Out-Null
git mv src/crawl/mod.rs crates/crawl/src/lib.rs
git mv src/crawl/auto.rs crates/crawl/src/auto.rs
git mv src/crawl/builder.rs crates/crawl/src/builder.rs
git mv src/crawl/engine.rs crates/crawl/src/engine.rs
git mv src/crawl/page.rs crates/crawl/src/page.rs
git mv src/crawl/runner.rs crates/crawl/src/runner.rs
git mv src/crawl/middleware crates/crawl/src/middleware
git mv src/crawl/observability crates/crawl/src/observability
git mv src/crawl/runtime crates/crawl/src/runtime
git mv src/crawl/scheduling crates/crawl/src/scheduling
```

`crates/crawl/Cargo.toml`：

```toml
[package]
name = "wisp-crawl"
version = "0.1.0"
edition = "2021"

[dependencies]
wisp-core = { path = "../core" }
wisp-fetcher = { path = "../fetcher" }
wisp-http = { path = "../http" }
wisp-parser = { path = "../parser" }
wisp-storage = { path = "../storage" }
wisp-proxy = { path = "../proxy" }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
async-trait = "0.1"
futures = "0.3"
tracing = "0.1"
regex = "1"
aho-corasick = "1"
moka = { version = "0.12", features = ["future", "sync"] }
dashmap = "6"
chrono = { version = "0.4", features = ["serde"] }
htmd = "0.5"
url = "2"
```

- [x] **Step 2: 修正 crate 内部引用**

对 `crates/crawl/src/` 下所有文件执行以下替换（用 `rg -l` 找到命中文件，逐个 `apply_patch`）：

| 旧路径 | 新路径 |
|---|---|
| `crate::error` | `wisp_core::error` |
| `crate::fetcher` | `wisp_fetcher` |
| `crate::http` | `wisp_http` |
| `crate::parser` | `wisp_parser` |
| `crate::storage` | `wisp_storage` |
| `crate::proxy` | `wisp_proxy` |
| `crate::utils` | `wisp_core::utils` |
| `crate::crawl` | `crate` |

关键点：
- `crates/crawl/src/lib.rs`：`pub use crate::fetcher::{Method, Request, Response};` -> `pub use wisp_core::{Method, Request, Response};`
- `crates/crawl/src/page.rs`：`use crate::fetcher::{Request, Response};` -> `use wisp_core::{Request, Response};`；`use crate::parser::{Node, NodeList};` -> `use wisp_parser::{Node, NodeList};`；`use crate::parser::ResponseExt;`（Page::new 调 `resp.parse()`）
- `crates/crawl/src/builder.rs`：`use crate::parser::Node;` -> `use wisp_parser::Node;`；`use wisp_parser::ResponseExt;`
- `crates/crawl/src/engine.rs`：`use crate::fetcher::FetchMode;` -> `use wisp_fetcher::FetchMode;`；`use crate::http::Client;` -> `use wisp_http::Client;`；`sanitize_url` 调用改为 `wisp_core::utils::sanitize_url` 并删除本地定义
- `crates/crawl/src/middleware/builtin.rs`：`use crate::crawl::auto` -> `use crate::auto`；`use crate::crawl::{Request, Response}` -> `use wisp_core::{Request, Response}`；`use crate::fetcher::FetchMode;` -> `use wisp_fetcher::FetchMode;`；`use crate::http::Client;` -> `use wisp_http::Client;`；`use crate::storage::{CachedResponse, Store};` -> `use wisp_storage::{CachedResponse, Store};`；`use crate::proxy::ProxyPool` -> `use wisp_proxy::ProxyPool`
- `crates/crawl/src/middleware/mod.rs`：`use crate::crawl::{Request, Response};` -> `use wisp_core::{Request, Response};`；`use crate::fetcher::FetchMode;` -> `use wisp_fetcher::FetchMode;`
- `crates/crawl/src/observability/events.rs`：`use crate::crawl::CrawlStats;` -> `use crate::CrawlStats;`；`use crate::fetcher::FetchMode;` -> `use wisp_fetcher::FetchMode;`
- `crates/crawl/src/observability/state.rs`：`use crate::crawl::{Request, CrawlStats};` -> `use crate::{CrawlStats, Request};`
- `crates/crawl/src/runtime/output.rs`：`use crate::utils::{status_text, url_to_filename};` -> `use wisp_core::utils::{status_text, url_to_filename};`
- `crates/crawl/src/runtime/robots.rs`：`use crate::http::Client;` -> `use wisp_http::Client;`
- `crates/crawl/src/scheduling/scheduler.rs`：`use crate::crawl::Request;` -> `use wisp_core::Request;`
- `crates/crawl/src/lib.rs` 底部测试模块中 `use crate::utils::resolve_href;` -> `use wisp_core::utils::resolve_href;`；`resp.parse()` 调用加 `use wisp_parser::ResponseExt;`

- [x] **Step 3: 建立根 shim**

`src/crawl/mod.rs` 整文件替换为：

```rust
//! Spider-based crawling engine（re-export 自 wisp-crawl）。
pub use wisp_crawl::*;
```

根 `Cargo.toml`：`members` 增加 `"crates/crawl"`，`[dependencies]` 增加 `wisp-crawl = { path = "crates/crawl" }`。

- [x] **Step 4: 编译并跑测试**

```bash
cargo test --workspace
```

Expected: 通过（`novel_10_books_test`、`multi_spider_test` 等全部继续通过）。

- [x] **Step 5: 提交**

```bash
git add -A
git commit -m "refactor(crawl): extract wisp-crawl crate"
```

---

## Task 10: wisp-mcp 拆分

**Files:**
- Move: `src/mcp/mod.rs`, `src/mcp/tools.rs` -> `crates/mcp/src/`
- Create: `crates/mcp/Cargo.toml`
- Replace: `src/mcp/mod.rs`（shim）
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `wisp_core`、`wisp_crawl::Engine`、`wisp_storage::Store`、`wisp_parser::Node`、`wisp_http::Client`、`wisp_browser::Browser`。
- Produces: `wisp_mcp::serve`、`wisp_mcp::tools`。

- [x] **Step 1: 创建 crate 并移动文件**

```bash
New-Item -ItemType Directory -Force crates/mcp/src | Out-Null
git mv src/mcp/mod.rs crates/mcp/src/lib.rs
git mv src/mcp/tools.rs crates/mcp/src/tools.rs
```

`crates/mcp/Cargo.toml`：

```toml
[package]
name = "wisp-mcp"
version = "0.1.0"
edition = "2021"

[dependencies]
wisp-core = { path = "../core" }
wisp-crawl = { path = "../crawl" }
wisp-storage = { path = "../storage" }
wisp-parser = { path = "../parser" }
wisp-http = { path = "../http" }
wisp-browser = { path = "../browser" }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
wreq-util = "=3.0.0-rc.14"
```

- [x] **Step 2: 修正 crate 内部引用**

`crates/mcp/src/lib.rs`、`tools.rs`：
- `use crate::error::{...};` -> `use wisp_core::error::{...};`
- `use crate::storage::Store;` -> `use wisp_storage::Store;`
- `use crate::crawl::Engine;` -> `use wisp_crawl::Engine;`
- `use crate::parser::Node;` -> `use wisp_parser::Node;`
- `use crate::http::Client;` -> `use wisp_http::Client;`
- `use crate::utils::validate_url` -> `use wisp_core::utils::validate_url;`
- `tools.rs` 测试中 `use crate::{Browser, LaunchOptions};` -> `use wisp_browser::Browser; use wisp_core::config::LaunchOptions;`
- `tools.rs` 测试中 `use crate::crawl::{Spider, Request, Response, MaxPages, StopCondition};` -> `use wisp_crawl::{MaxPages, Spider, StopCondition}; use wisp_core::{Request, Response};`
- `tools.rs` 测试中 `use crate::parser::css_adaptive;` -> `use wisp_parser::css_adaptive;`

- [x] **Step 3: 建立根 shim**

`src/mcp/mod.rs` 整文件替换为：

```rust
//! MCP server（re-export 自 wisp-mcp）。
pub use wisp_mcp::*;
```

根 `Cargo.toml`：`members` 增加 `"crates/mcp"`，`[dependencies]` 增加 `wisp-mcp = { path = "crates/mcp" }`。

- [x] **Step 4: 编译并跑测试**

```bash
cargo test --workspace
```

Expected: 通过。

- [x] **Step 5: 提交**

```bash
git add -A
git commit -m "refactor(mcp): extract wisp-mcp crate"
```

---

## Task 11: facade 收尾（删除 shim，统一 re-export）

**Files:**
- Delete: `src/error.rs`, `src/config.rs`, `src/text.rs`, `src/utils/`, `src/http/`, `src/storage/`, `src/parser/`, `src/proxy.rs`, `src/config_file.rs`, `src/browser/`, `src/stealth/`, `src/fetcher/`, `src/crawl/`, `src/mcp/`
- Rewrite: `src/lib.rs`
- Modify: `Cargo.toml`（根依赖收敛到 facade 需要）

**Interfaces:**
- Produces: 最终 facade，公共路径 `wisp::crawl::...`、`wisp::fetcher::...`、`wisp::storage::...` 全部保持。

- [x] **Step 1: 删除根 shim 目录/文件**

```powershell
Remove-Item -LiteralPath (Resolve-Path src/error.rs, src/config.rs, src/text.rs, src/utils, src/http, src/storage, src/parser, src/proxy.rs, src/config_file.rs, src/browser, src/stealth, src/fetcher, src/crawl, src/mcp) -Recurse -Force
```

执行前用 `Resolve-Path` 确认每个目标都在 `F:\project\wisp\src` 内。

- [x] **Step 2: 重写 src/lib.rs**

```rust
//! wisp: Lightweight undetected browser automation for Rust.
//!
//! Pure Rust CDP (Chrome DevTools Protocol) over WebSocket with anti-detection
//! patches. Built for scraping — passes Browserscan 4/4 in both headed and headless.
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use wisp::{Fetcher, parser::ResponseExt};
//!
//! # async fn example() -> wisp::Result<()> {
//! let page = Fetcher::http().get("https://quotes.toscrape.com/").await?;
//! let quotes = page.css(".quote .text");
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)]

// 子 crate 按模块路径 re-export，保持既有 `wisp::xxx` API。
pub use wisp_browser as browser;
pub use wisp_crawl as crawl;
pub use wisp_fetcher as fetcher;
pub use wisp_http as http;
pub use wisp_mcp as mcp;
pub use wisp_parser as parser;
pub use wisp_proxy as proxy;
pub use wisp_stealth as stealth;
pub use wisp_storage as storage;
pub use wisp_core::{config, error, text, utils};

// config_file 位于 wisp-proxy crate，单独保留旧路径 `wisp::config_file`。
pub mod config_file {
    pub use wisp_proxy::config_file::*;
}

// === 统一入口 ===
pub use fetcher::{FetchClient, FetchClientConfig, Fetcher, FetchMode, FetcherBuilder};
pub use fetcher::{Response, Request, Method};
pub use stealth::TurnstileConfig;

// === 核心类型 ===
pub use browser::{Browser, Page};
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{WispError, Result, BrowserError, NetworkError, ParseError, McpError, StorageError};

pub use parser::{Node, NodeList, ResponseExt};
pub use proxy::RotationStrategy;
pub use storage::{Store, MemoryStore, FileStore, CachedResponse, ElementSnapshotRow};

// 自由函数导出（业务层 API）
pub use storage::{
    save_checkpoint, load_checkpoint, delete_checkpoint,
    save_element, load_element,
    save_response, load_response, delete_response,
};

#[cfg(feature = "sqlite")]
pub use storage::SqliteStore;

// === 爬虫引擎 ===
pub use crawl::{Spider, Engine, CrawlEvent, CrawlStream, Items, JsonlWriter, SpiderBuilder, ClosureSpider};
pub use http::UaRotator;

// === 底层类型（FetchClientConfig 公共字段需要） ===
pub use http::DomainBlocker;
```

- [x] **Step 3: 收敛根 Cargo.toml**

根 `[dependencies]` 只保留：

```toml
wisp-core = { path = "crates/core" }
wisp-storage = { path = "crates/storage" }
wisp-parser = { path = "crates/parser" }
wisp-proxy = { path = "crates/proxy" }
wisp-http = { path = "crates/http" }
wisp-browser = { path = "crates/browser" }
wisp-stealth = { path = "crates/stealth" }
wisp-fetcher = { path = "crates/fetcher" }
wisp-crawl = { path = "crates/crawl" }
wisp-mcp = { path = "crates/mcp" }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
wreq-util = "=3.0.0-rc.14"
```

`[profile.release]`、`[profile.profiling]`、`[[bin]]`、`[[bench]]`、`[dev-dependencies]` 保持原样。

- [x] **Step 4: 更新 README 架构段**

把 README 的 `## Architecture` 中 `src/` 结构替换为 workspace 结构，注明每个 crate 职责与依赖方向；删除不存在的 `session.rs`、`request_cache.rs`、`SpiderRequest` 引用。

README 中所有直接对 `Response` 调用 `.css()`/`.parse()` 的示例，在文件头加 `use wisp::parser::ResponseExt;`。

- [x] **Step 5: 全量验证**

```bash
cargo test --workspace --all-targets
```

Expected: 全部通过。

- [x] **Step 6: 提交**

```bash
git add -A
git commit -m "refactor(facade): collapse root modules to workspace re-exports"
```

---

## Task 12: 全量验证与依赖图断言

**Files:**
- Modify: `deny.toml`（如需补充 crate 级禁止依赖）
- Test: 新增 `scripts/check_deps.ps1`（可选，作为验收工具）

- [x] **Step 1: 编译与测试全绿**

```bash
cargo build --workspace
cargo test --workspace
cargo bench --bench bench -- novel_flow
```

Expected: build/test 通过；bench 可运行（数值允许波动）。

- [x] **Step 2: 断言依赖方向无环**

创建 `scripts/check_deps.ps1`：

```powershell
$bad = rg -n "wisp_fetcher|wisp-fetcher" crates/http crates/core crates/storage crates/parser crates/proxy crates/browser
$bad += rg -n "wisp_crawl|wisp-crawl" crates/fetcher crates/http crates/core crates/storage crates/parser crates/proxy crates/browser crates/stealth
if ($bad) { $bad; exit 1 }
Write-Host "dependency direction OK"
```

运行：

```powershell
powershell -File scripts/check_deps.ps1
```

Expected: `dependency direction OK`。

- [x] **Step 3: 验证 facade 公共路径**

```bash
cargo check --examples
cargo check --tests
```

Expected: examples/tests 无需改动即可编译（依赖 `wisp::` facade）。

- [x] **Step 4: 提交**

```bash
git add -A
git commit -m "test(deps): add workspace dependency direction check"
```

---

## 后续计划（本计划完成后另行创建）

1. `2026-07-31-routing-errors-isolation.md`：`Request.spider` 改稳定 id、`Spider::handle` 返回 `Result`、中间件动作按阶段类型化、单页 panic 隔离、事件背压统一。
2. `2026-07-31-checkpoint-resume.md`：run 前恢复 checkpoint、per-spider 保存/恢复、in-flight 丢失修复、中断恢复验收。
3. `2026-07-31-public-surface-cleanup.md`：接线或删除 SessionPool/EventBus/Output 未用部分，收敛传输配置。

## 自审记录

- 规格覆盖：crate 拆分（Task 0-10）、依赖方向断言（Task 5/8/12）、facade 公共路径（Task 11）、README 修正（Task 11）。
- 占位符扫描：无 TBD；所有文件移动给出精确路径，所有行为变更给出代码。
- 类型一致性：`ResponseExt` 从 Task 1 定义后，Task 3 整体搬入 `wisp-parser`，Task 9 的 crawl 继续从 `wisp_parser` 导入，签名未变。
