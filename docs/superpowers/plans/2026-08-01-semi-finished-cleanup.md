# wisp 半成品与冗余修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复第二轮 code review 发现的 18 项半成品、死代码与边界缺陷，使公开承诺的功能真实生效、无界增长有界、存储/统计恢复完整。

**Architecture:** 分 12 个独立提交任务，按“先补全 MCP 共享能力，再接抓取能力，再修配置边界，最后统计/增长/清理”的顺序执行。每个任务先写失败测试或编译失败用例，再做最小实现，最后跑针对性验证。

**Tech Stack:** Rust 2021 workspace、cargo、wreq 6.0.0-rc.29（启用 `socks`）、tokio、serde/bincode、dashmap、regex、serde_json。

## Global Constraints

- 所有命令在仓库根目录 `F:\project\wisp` 执行。
- 变量命名 snake_case；提交信息使用中文。
- 编码前加载 `rust-best-practices` 与 `rust-async-patterns` 技能；遇到失败用 `systematic-debugging`。
- 真实网络/Chrome 测试一律 `#[ignore]`，不纳入自动验证。
- 公共 API 变更仅限：新增 `EngineBuilder::fetch_client`、`DomainBlocker::blocked_domains()`、MCP CLI `--headless`/`--human-mode`、`CrawlState` 五个统计字段；变更 `FetchClient::build_browser_pool`、`push_proxy_arg`、`build_stealth_args`/`build_default_args`、`Fetcher::from_client` 返回 `Result`、MCP `serve` 签名；移除 `Config.header_order`、`http::proxy` 模块、`UaRotator`、`CrawlState::from_stats`、`Browser.user_data_path`、`stealth_fetch.headless/human_mode` 参数。
- 每个任务只 `git add` 本任务涉及的文件；工作树其他未提交改动不得纳入提交。
- 每个任务结束时必须运行“验证”一节列出的命令，全部通过才提交。

---

### Task 1: MCP 共享 FetchClient 与 stealth_fetch 真实接线

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/mcp/Cargo.toml`
- Modify: `crates/mcp/src/protocol.rs`
- Modify: `crates/mcp/src/server.rs`
- Modify: `crates/mcp/src/tools/stealth.rs`
- Modify: `crates/mcp/src/tools/mod.rs`
- Modify: `crates/mcp/src/tests.rs`
- Modify: `crates/crawl/src/runner/builder/mod.rs`
- Modify: `crates/crawl/src/runner/builder/methods.rs`
- Modify: `crates/crawl/src/runner/builder/build.rs`
- Modify: `crates/crawl/src/runner/tests.rs`
- Modify: `src/bin/wisp/main.rs`
- Modify: `src/bin/wisp/mcp.rs`

**Interfaces:**
- Consumes: `FetchClient`、`FetchClientConfig`、`EngineBuilder`。
- Produces: `serve(store: Arc<dyn Store>, fetch_client: Arc<FetchClient>)`；`tools::stealth_fetch(args, &Arc<FetchClient>)`；`EngineBuilder::fetch_client(Arc<FetchClient>)`；MCP CLI `--headless`/`--human-mode`；wisp-mcp `stealth` feature。

- [ ] **Step 1: 写失败测试**

`crates/crawl/src/runner/tests.rs` 追加：

```rust
#[test]
fn engine_builder_accepts_existing_fetch_client() {
    use std::sync::Arc;
    use wisp_fetcher::{FetchClient, FetchClientConfig};

    let client = Arc::new(FetchClient::new(FetchClientConfig::default()).unwrap());
    let engine = Engine::infra()
        .fetch_client(Arc::clone(&client))
        .build()
        .unwrap();
    assert!(Arc::ptr_eq(&engine.fetch_client, &client));
}
```

`crates/mcp/src/tests.rs` 修改 `test_handle_tools_call_unknown_tool`，并新增 stealth 用例：

```rust
use wisp_fetcher::{FetchClient, FetchClientConfig};

fn test_fetch_client() -> Arc<FetchClient> {
    Arc::new(
        FetchClient::new(FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        })
        .expect("build test fetch client"),
    )
}

#[tokio::test]
async fn test_handle_tools_call_unknown_tool() {
    let store: Arc<dyn Store> = Arc::new(wisp_storage::MemoryStore::default());
    let engine = Engine::infra()
        .max_pages(100)
        .obey_robots(false)
        .build()
        .unwrap();
    let req = json!({
        "params": { "name": "nonexistent", "arguments": {} }
    });
    let result = handle_tools_call(req, &store, &engine, &test_fetch_client()).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        WispError::Mcp(McpError::UnknownTool(n)) => assert_eq!(n, "nonexistent"),
        other => panic!("预期 McpUnknownTool, 得到 {:?}", other),
    }
}

#[cfg(feature = "stealth")]
#[tokio::test]
async fn stealth_fetch_requires_url() {
    let client = test_fetch_client();
    let result = crate::tools::stealth_fetch(json!({}), &client).await;
    let err = result.expect_err("缺少 url 应报错");
    assert!(err.to_string().contains("url"), "错误应说明缺少 url: {err}");
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p wisp-crawl --lib runner::tests::engine_builder_accepts_existing_fetch_client`
Expected: FAIL，`fetch_client` 方法不存在。

Run: `cargo test -p wisp-mcp --lib --all-features`
Expected: FAIL，`handle_tools_call` 参数数量不匹配、`stealth_fetch` 签名不匹配。

- [ ] **Step 3: 实现**

`Cargo.toml` 的 `stealth` feature 增加转发：

```toml
stealth = ["browser", "wisp-fetcher/stealth", "wisp-crawl/stealth", "dep:wisp-stealth", "wisp-mcp?/stealth"]
```

`crates/mcp/Cargo.toml` 的 `[features]` 与依赖改为：

```toml
[features]
default = ["browser"]
browser = ["wisp-fetcher/browser"]
stealth = ["wisp-fetcher/stealth"]
sqlite = ["wisp-storage/sqlite"]

[dependencies]
wisp-core = { path = "../core" }
wisp-crawl = { path = "../crawl" }
wisp-fetcher = { path = "../fetcher" }
wisp-storage = { path = "../storage" }
wisp-parser = { path = "../parser" }
wisp-http = { path = "../http" }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
wreq-util = "=3.0.0-rc.14"
```

删除 `wisp-browser = { path = "../browser", optional = true }`。

`crates/crawl/src/runner/builder/mod.rs`：

```rust
pub struct EngineBuilder {
    fetch_client: Option<Arc<FetchClient>>,
    fetch_client_config: FetchClientConfig,
    // ...其余字段不变
}
```

`infra()` 初始化加 `fetch_client: None,`。

`crates/crawl/src/runner/builder/methods.rs` 新增：

```rust
    /// 使用已有 FetchClient（MCP 等需要共享 HTTP/BrowserPool 的场景）。
    pub fn fetch_client(mut self, client: Arc<FetchClient>) -> Self {
        self.fetch_client = Some(client);
        self
    }
```

`crates/crawl/src/runner/builder/build.rs` 的 `build()` 开头改为：

```rust
    pub fn build(self) -> Result<Engine> {
        if self.max_concurrent == 0 {
            return Err(wisp_core::error::WispError::Config(
                "max_concurrent must be > 0".into(),
            ));
        }
        let fetch_client = match self.fetch_client {
            Some(client) => client,
            None => Arc::new(FetchClient::new(self.fetch_client_config)?),
        };
        Ok(Engine {
            fetch_client,
            // ...其余字段不变
        })
    }
```

`crates/mcp/src/tools/stealth.rs` 整体替换为：

```rust
//! MCP stealth_fetch 工具：复用共享 FetchClient + StealthStrategy。

#[cfg(feature = "stealth")]
use serde_json::{json, Value};
#[cfg(feature = "stealth")]
use std::sync::Arc;
#[cfg(feature = "stealth")]
use wisp_core::error::{McpError, Result, WispError};
#[cfg(feature = "stealth")]
use wisp_core::Request;
#[cfg(feature = "stealth")]
use wisp_fetcher::{cookie::CfCookieJar, FetchClient, StealthStrategy};

#[cfg(feature = "stealth")]
pub async fn stealth_fetch(args: Value, fetch_client: &Arc<FetchClient>) -> Result<Value> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'url'".into())))?;
    wisp_core::utils::validate_url(url)?;
    let config = fetch_client.config();
    let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
    let strategy = StealthStrategy::from_config(config, cf_jar);
    let req = Request::get(url);
    let resp = fetch_client
        .fetch_browser(&req, &strategy)
        .await
        .map_err(|e| WispError::Mcp(McpError::General(format!("stealth fetch: {e}"))))?;
    let html = String::from_utf8_lossy(&resp.body).to_string();
    Ok(json!({
        "url": resp.url,
        "title": resp.title,
        "html": html,
        "bytes": resp.body.len()
    }))
}
```

`crates/mcp/src/tools/mod.rs`：

```rust
#[cfg(feature = "stealth")]
mod stealth;

#[cfg(feature = "stealth")]
pub use stealth::stealth_fetch;
```

`crates/mcp/src/protocol.rs` 的 stealth 工具改为：

```rust
        #[cfg(feature = "stealth")]
        Tool {
            name: "stealth_fetch",
            description: "浏览器隐身抓取（CF 挑战解决 + 人类行为模拟，复用共享浏览器池）。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string" }
                },
                "required": ["url"]
            }),
        },
```

`crates/mcp/src/server.rs`：

```rust
use wisp_fetcher::FetchClient;

async fn dispatch_request(
    request: Value,
    store: &Arc<dyn Store>,
    engine: &Engine,
    fetch_client: &Arc<FetchClient>,
) -> Value {
    // ...
    "tools/call" => match handle_tools_call(request, store, engine, fetch_client).await {
        // ...
    },
    // ...
}

async fn handle_request_line(
    line: &str,
    store: &Arc<dyn Store>,
    engine: &Engine,
    fetch_client: &Arc<FetchClient>,
    stdout: &mut tokio::io::Stdout,
) -> Result<()> {
    // ...
    let response = dispatch_request(request, store, engine, fetch_client).await;
    // ...
}

pub async fn serve(store: Arc<dyn Store>, fetch_client: Arc<FetchClient>) -> Result<()> {
    let engine = Engine::infra()
        .fetch_client(fetch_client.clone())
        .max_pages(100000)
        .obey_robots(false)
        .build()?;
    // ...
    while let Some(line) = lines.next_line().await? {
        handle_request_line(&line, &store, &engine, &fetch_client, &mut stdout).await?;
    }
    Ok(())
}

pub(super) async fn handle_tools_call(
    request: Value,
    store: &Arc<dyn Store>,
    engine: &Engine,
    fetch_client: &Arc<FetchClient>,
) -> Result<Value> {
    // ...
    let result = match name {
        // ...
        #[cfg(feature = "stealth")]
        "stealth_fetch" => tools::stealth_fetch(args, fetch_client).await,
        // ...
    }?;
    // ...
}
```

`src/bin/wisp/main.rs` 的 `McpCmd::Serve`：

```rust
    Serve {
        #[arg(long, default_value = "./wisp.db")]
        db: String,
        #[arg(long, default_value_t = true)]
        headless: bool,
        #[arg(long, default_value_t = true)]
        human_mode: bool,
    },
```

`src/bin/wisp/mcp.rs`：

```rust
use std::sync::Arc;

pub(super) async fn run_mcp(cmd: McpCmd) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        McpCmd::Serve {
            db,
            headless,
            human_mode,
        } => {
            let store = build_store(&db)?;
            let fetch_client = Arc::new(wisp::FetchClient::new(wisp::FetchClientConfig {
                headless,
                human_mode,
                ..Default::default()
            })?);
            wisp::mcp::serve(store, fetch_client).await?;
        }
    }
    Ok(())
}
```

`crates/mcp/src/tests.rs` 的工具数量断言改为：

```rust
#[cfg(feature = "stealth")]
assert_eq!(tools.len(), 5, "stealth feature 下应有 5 个工具");
#[cfg(not(feature = "stealth"))]
assert_eq!(tools.len(), 4, "无 stealth feature 时应为 4 个工具");
#[cfg(feature = "stealth")]
assert!(names.contains(&"stealth_fetch"));
```

- [ ] **Step 4: 验证**

Run: `cargo test -p wisp-crawl --lib runner::tests::engine_builder_accepts_existing_fetch_client`
Expected: PASS。

Run: `cargo test -p wisp-mcp --lib --all-features`
Expected: PASS。

Run: `cargo check --workspace --all-features --all-targets`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add Cargo.toml crates/mcp/Cargo.toml crates/mcp/src/protocol.rs crates/mcp/src/server.rs crates/mcp/src/tools/stealth.rs crates/mcp/src/tools/mod.rs crates/mcp/src/tests.rs crates/crawl/src/runner/builder/mod.rs crates/crawl/src/runner/builder/methods.rs crates/crawl/src/runner/builder/build.rs crates/crawl/src/runner/tests.rs src/bin/wisp/main.rs src/bin/wisp/mcp.rs Cargo.lock
git commit -m "feat: MCP 共享 FetchClient 并真实接入 stealth_fetch"
```

---

### Task 2: crawl_site 链接跟随聚焦扩展

**Files:**
- Modify: `crates/mcp/Cargo.toml`
- Modify: `crates/mcp/src/tools/spider.rs`
- Modify: `crates/mcp/src/tools/crawl.rs`
- Modify: `crates/mcp/src/protocol.rs`
- Test: `crates/mcp/src/tools/mod.rs`

**Interfaces:**
- Consumes: `wisp_core::utils::resolve_href`、`Request::with_spider/with_depth/with_callback`。
- Produces: `SimpleSpider` 新增 `follow_pattern: Option<Regex>`、`max_depth: usize`、`allowed_domains: Vec<String>`；`crawl_site` 读取 `follow_pattern`/`max_depth`/`allowed_domains`。

- [ ] **Step 1: 写失败测试**

`crates/mcp/src/tools/spider.rs` 追加测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    #[tokio::test]
    async fn follows_matching_links_with_depth_and_domain() {
        let spider = SimpleSpider {
            css: "p".into(),
            start_urls: vec!["https://example.com/".into()],
            max_pages: 10,
            follow_pattern: Some(Regex::new(r"^https://example\.com/blog/").unwrap()),
            max_depth: 1,
            allowed_domains: vec!["example.com".into()],
        };
        let resp = Response::from_browser(
            200,
            "https://example.com/".into(),
            r#"<a href="/blog/1">b</a><a href="/other">o</a><a href="https://evil.com/x">e</a>"#.into(),
            "t".into(),
            vec![],
            Request::get("https://example.com/"),
        );
        let (_items, follows) = spider.handle(resp).await;
        assert_eq!(follows.len(), 1);
        assert_eq!(follows[0].url, "https://example.com/blog/1");
        assert_eq!(follows[0].depth, 1);
        assert_eq!(follows[0].spider.as_deref(), Some("mcp_simple"));
    }

    #[tokio::test]
    async fn respects_max_depth() {
        let spider = SimpleSpider {
            css: "p".into(),
            start_urls: vec!["https://example.com/".into()],
            max_pages: 10,
            follow_pattern: None,
            max_depth: 1,
            allowed_domains: vec![],
        };
        let mut req = Request::get("https://example.com/");
        req.depth = 1;
        let resp = Response::from_browser(
            200,
            "https://example.com/".into(),
            r#"<a href="/next">x</a>"#.into(),
            "t".into(),
            vec![],
            req,
        );
        let (_items, follows) = spider.handle(resp).await;
        assert!(follows.is_empty(), "depth 1 + max_depth 1 不应继续跟随");
    }
}
```

`crates/mcp/src/tools/mod.rs` 测试模块追加：

```rust
    #[tokio::test]
    async fn crawl_site_rejects_invalid_follow_pattern() {
        let engine = Engine::infra()
            .max_pages(0)
            .obey_robots(false)
            .build()
            .unwrap();
        let args = json!({
            "start_urls": ["https://example.com/"],
            "css_selector": "p",
            "follow_pattern": "["
        });
        let result = crawl_site(args, &engine).await;
        assert!(result.is_err(), "非法正则应报错");
    }
```

（`tools/mod.rs` 测试模块需补 `use wisp_crawl::Engine;`。）

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p wisp-mcp --lib tools::tests::follows_matching_links_with_depth_and_domain`
Expected: FAIL，`follow_pattern`/`max_depth`/`allowed_domains` 字段不存在。

Run: `cargo test -p wisp-mcp --lib tools::tests::crawl_site_rejects_invalid_follow_pattern`
Expected: FAIL（当前忽略 `follow_pattern`，`max_pages(0)` 下返回 Ok）。

- [ ] **Step 3: 实现**

`crates/mcp/Cargo.toml` 的 `[dependencies]` 加：

```toml
regex = "1"
```

`crates/mcp/src/tools/spider.rs`：

```rust
//! MCP SimpleSpider。

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::Arc;
use wisp_core::{Request, Response};
use wisp_crawl::{MaxPages, Spider, StopCondition};
use wisp_parser::Node;

pub(super) struct SimpleSpider {
    pub(super) css: String,
    pub(super) start_urls: Vec<String>,
    pub(super) max_pages: usize,
    pub(super) follow_pattern: Option<Regex>,
    pub(super) max_depth: u32,
    pub(super) allowed_domains: Vec<String>,
}

#[async_trait]
impl Spider for SimpleSpider {
    fn name(&self) -> &str {
        "mcp_simple"
    }
    fn start_urls(&self) -> Vec<String> {
        self.start_urls.clone()
    }
    async fn handle(&self, resp: Response) -> (Vec<Value>, Vec<Request>) {
        let text = resp.text().unwrap_or_default();
        let doc = Node::from_html(&text);
        let nodes = doc.select(&self.css);
        let items: Vec<Value> = nodes
            .iter()
            .map(|n| json!({"text": n.text(), "html": n.html()}))
            .collect();

        let page_url = resp.url.clone();
        let spider_name = resp
            .request
            .spider
            .clone()
            .unwrap_or_else(|| self.name().to_string());
        let current_depth = resp.request.depth;
        let mut follows = Vec::new();
        for link in doc.select("a[href]") {
            let Some(href) = link.attr("href") else {
                continue;
            };
            let Some(url) = wisp_core::utils::resolve_href(&page_url, &href) else {
                continue;
            };
            if let Some(ref re) = self.follow_pattern {
                if !re.is_match(&url) {
                    continue;
                }
            }
            if self.max_depth > 0 && current_depth + 1 > self.max_depth {
                continue;
            }
            if !self.allowed_domains.is_empty() {
                let host = url::Url::parse(&url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_string))
                    .unwrap_or_default();
                let allowed = self
                    .allowed_domains
                    .iter()
                    .any(|d| host == *d || host.ends_with(&format!(".{d}")));
                if !allowed {
                    continue;
                }
            }
            let mut req = Request::get(&url)
                .with_spider(spider_name.clone())
                .with_depth(current_depth + 1);
            if let Some(ref cb) = resp.request.callback {
                req = req.with_callback(cb.as_str());
            }
            follows.push(req);
        }
        (items, follows)
    }
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(MaxPages(self.max_pages))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    #[tokio::test]
    async fn follows_matching_links_with_depth_and_domain() {
        // ...Step 1 测试代码
    }

    #[tokio::test]
    async fn respects_max_depth() {
        // ...Step 1 测试代码
    }
}
```

`crates/mcp/src/tools/crawl.rs` 在构造 spider 前解析新参数：

```rust
use regex::Regex;

    let follow_pattern = args
        .get("follow_pattern")
        .and_then(|v| v.as_str())
        .map(|p| {
            Regex::new(p).map_err(|e| {
                WispError::Mcp(McpError::General(format!(
                    "invalid follow_pattern regex: {e}"
                )))
            })
        })
        .transpose()?;
    let max_depth = args
        .get("max_depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let allowed_domains: Vec<String> = args
        .get("allowed_domains")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let spider = SimpleSpider {
        css: css_selector,
        start_urls,
        max_pages,
        follow_pattern,
        max_depth,
        allowed_domains,
    };
```

`crates/mcp/src/protocol.rs` 的 `crawl_site` schema 扩展：

```rust
            input_schema: json!({
                "type": "object",
                "properties": {
                    "start_urls": { "type": "array", "items": { "type": "string" } },
                    "css_selector": { "type": "string", "description": "每页提取的 CSS 选择器" },
                    "max_pages": { "type": "integer", "default": 100 },
                    "follow_pattern": { "type": "string", "description": "可选：仅跟随匹配此正则的链接" },
                    "max_depth": { "type": "integer", "default": 0, "description": "最大跟随深度，0 表示不限制" },
                    "allowed_domains": { "type": "array", "items": { "type": "string" }, "description": "可选：仅跟随这些域名的链接" }
                },
                "required": ["start_urls", "css_selector"]
            }),
```

- [ ] **Step 4: 验证**

Run: `cargo test -p wisp-mcp --lib`
Expected: PASS。

Run: `cargo test -p wisp-mcp --all-features --lib`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/mcp/Cargo.toml crates/mcp/src/tools/spider.rs crates/mcp/src/tools/crawl.rs crates/mcp/src/protocol.rs crates/mcp/src/tools/mod.rs Cargo.lock
git commit -m "feat: crawl_site 支持 follow_pattern/max_depth/allowed_domains 真实跟随链接"
```

---

### Task 3: adaptive_scrape db_path 与 MCP CLI --db 语义

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/mcp/Cargo.toml`
- Modify: `crates/mcp/src/tools/adaptive.rs`
- Modify: `src/bin/wisp/mcp.rs`
- Test: `crates/mcp/src/tools/adaptive.rs`

**Interfaces:**
- Produces: `store_from_db_path(db_path: &str) -> Result<Arc<dyn Store>>`（模块私有）；`adaptive_scrape` 的 `db_path` 参数生效；无 sqlite 时 CLI `--db` 使用 `FileStore::with_dir` 并提示。

- [ ] **Step 1: 写失败测试**

`crates/mcp/src/tools/adaptive.rs` 追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_path_creates_expected_backend() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshots.db");
        let _store = store_from_db_path(path.to_str().unwrap()).unwrap();
        #[cfg(feature = "sqlite")]
        assert!(path.is_file(), "sqlite 应创建 db 文件");
        #[cfg(not(feature = "sqlite"))]
        assert!(path.is_dir(), "无 sqlite 应创建 FileStore 目录");
    }
}
```

`crates/mcp/Cargo.toml` 的 `[dev-dependencies]` 加：

```toml
tempfile = "3"
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p wisp-mcp --lib tools::adaptive::tests::db_path_creates_expected_backend`
Expected: FAIL，`store_from_db_path` 不存在。

- [ ] **Step 3: 实现**

`Cargo.toml` 的 `sqlite` feature 改为：

```toml
sqlite = ["wisp-storage/sqlite", "wisp-mcp?/sqlite"]
```

`crates/mcp/src/tools/adaptive.rs`：

```rust
//! MCP adaptive_scrape 工具。

use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use wisp_core::error::{McpError, Result, WispError};
use wisp_http::Client;
use wisp_parser::Node;
use wisp_storage::{FileStore, Store};
#[cfg(feature = "sqlite")]
use wisp_storage::SqliteStore;

#[cfg(feature = "sqlite")]
fn store_from_db_path(db_path: &str) -> Result<Arc<dyn Store>> {
    if db_path != ":memory:" {
        return Ok(Arc::new(SqliteStore::open(std::path::Path::new(db_path))?));
    }
    Ok(Arc::new(FileStore::default()))
}

#[cfg(not(feature = "sqlite"))]
fn store_from_db_path(db_path: &str) -> Result<Arc<dyn Store>> {
    if db_path == ":memory:" {
        return Ok(Arc::new(FileStore::default()));
    }
    Ok(Arc::new(FileStore::with_dir(PathBuf::from(db_path))))
}

async fn adaptive_scrape_impl(
    url: &str,
    selector: &str,
    key: &str,
    store: &Arc<dyn Store>,
) -> Result<Value> {
    wisp_core::utils::validate_url(url)?;
    let client = Client::builder().build()?;
    let resp = client.get(url, &[]).await?;
    let html = resp.text()?;
    let doc = Node::from_html(&html);
    use wisp_crawl::AdaptiveTracker;
    let tracker = AdaptiveTracker::new(Arc::clone(store));
    let found = tracker
        .css_adaptive(
            &doc,
            selector,
            key,
            url,
            true,
            wisp_parser::DEFAULT_TOLERANCE,
        )
        .await?;
    match found {
        Some(node) => Ok(json!({
            "url": url,
            "found": true,
            "text": node.text(),
            "html": node.html()
        })),
        None => Ok(json!({ "url": url, "found": false })),
    }
}

pub async fn adaptive_scrape(args: Value, store: &Arc<dyn Store>) -> Result<Value> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'url'".into())))?;
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'selector'".into())))?;
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'key'".into())))?;
    let db_path = args
        .get("db_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let effective_store: Arc<dyn Store> = match db_path {
        Some(path) => store_from_db_path(path)?,
        None => Arc::clone(store),
    };
    adaptive_scrape_impl(url, selector, key, &effective_store).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_path_creates_expected_backend() {
        // ...Step 1 测试代码
    }
}
```

`src/bin/wisp/mcp.rs` 的无 sqlite 分支改为：

```rust
    #[cfg(not(feature = "sqlite"))]
    {
        if db.is_empty() || db == ":memory:" {
            return Ok(Arc::new(wisp::FileStore::default()));
        }
        eprintln!("当前构建未启用 sqlite，使用 FileStore 目录: {db}");
        Ok(Arc::new(wisp::FileStore::with_dir(std::path::PathBuf::from(db))))
    }
```

- [ ] **Step 4: 验证**

Run: `cargo test -p wisp-mcp --lib tools::adaptive::tests::db_path_creates_expected_backend`
Expected: PASS。

Run: `cargo test -p wisp-mcp --all-features --lib`
Expected: PASS。

Run: `cargo check --workspace --no-default-features --all-targets`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add Cargo.toml crates/mcp/Cargo.toml crates/mcp/src/tools/adaptive.rs src/bin/wisp/mcp.rs Cargo.lock
git commit -m "feat: adaptive_scrape 的 db_path 真实接管存储并修正 MCP --db 语义"
```

---

### Task 4: DomainBlocker HTTP/浏览器接线

**Files:**
- Modify: `crates/http/src/block/blocker.rs`
- Modify: `crates/http/src/block/mod.rs`
- Modify: `crates/fetcher/src/client/fetch_client.rs`
- Modify: `crates/fetcher/src/client/mod.rs`

**Interfaces:**
- Produces: `DomainBlocker::blocked_domains() -> Vec<String>`；`FetchClient::fetch_http`/`fetch_browser` 命中时返回 `WispError::Config`。

- [ ] **Step 1: 写失败测试**

`crates/http/src/block/mod.rs` 测试模块追加：

```rust
    #[test]
    fn blocked_domains_returns_wildcard_patterns() {
        let mut blocker = DomainBlocker::new();
        blocker.block_domains(&["ads.example.com", "tracker.io"]);
        let mut patterns = blocker.blocked_domains();
        patterns.sort();
        assert_eq!(patterns, vec!["*://ads.example.com/*", "*://tracker.io/*"]);
    }
```

`crates/fetcher/src/client/mod.rs` 测试模块追加：

```rust
    #[tokio::test]
    async fn fetch_http_blocks_configured_domain() {
        use wisp_core::Request;
        use wisp_http::DomainBlocker;
        let mut blocker = DomainBlocker::new();
        blocker.block_domain("ads.example.com");
        let config = FetchClientConfig {
            domain_blocker: Some(blocker),
            ..Default::default()
        };
        let client = FetchClient::new(config).expect("build client");
        let err = client
            .fetch_http(&Request::get("https://ads.example.com/ad.js"))
            .await
            .expect_err("拦截域名应报错");
        assert!(err.to_string().contains("blocked"), "错误应说明拦截: {err}");
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p wisp-http --lib block::tests::blocked_domains_returns_wildcard_patterns`
Expected: FAIL，方法不存在。

Run: `cargo test -p wisp-fetcher --lib client::tests::fetch_http_blocks_configured_domain`
Expected: FAIL（当前会尝试发请求或返回网络错误，不返回 blocked）。

- [ ] **Step 3: 实现**

`crates/http/src/block/blocker.rs` 在 `to_cdp_patterns` 后新增：

```rust
    /// 返回可直接用于 CDP `Network.setBlockedURLs` 的 URL 模式列表。
    pub fn blocked_domains(&self) -> Vec<String> {
        self.blocked
            .iter()
            .map(|domain| format!("*://{}/*", domain))
            .collect()
    }
```

`crates/fetcher/src/client/fetch_client.rs` 把 `WispError` 导入移出 feature gate：

```rust
use wisp_core::error::{Result, WispError};
```

`fetch_http` 改为：

```rust
    pub async fn fetch_http(&self, req: &Request) -> Result<Response> {
        if let Some(ref blocker) = self.config.domain_blocker {
            if blocker.should_block(&req.url) {
                return Err(WispError::Config(format!(
                    "domain blocked by DomainBlocker: {}",
                    wisp_core::utils::sanitize_url(&req.url)
                )));
            }
        }
        self.http.fetch(req).await
    }
```

`fetch_browser` 在 `let mut handle = pool.acquire().await?;` 之后、strategy 调用之前插入：

```rust
        if let Some(ref blocker) = self.config.domain_blocker {
            if blocker.should_block(&req.url) {
                return Err(WispError::Config(format!(
                    "domain blocked by DomainBlocker: {}",
                    wisp_core::utils::sanitize_url(&req.url)
                )));
            }
            let urls = blocker.blocked_domains();
            if !urls.is_empty() {
                handle
                    .page_mut()
                    .cmd("Network.setBlockedURLs", serde_json::json!({ "urls": urls }))
                    .await?;
            }
        }
```

- [ ] **Step 4: 验证**

Run: `cargo test -p wisp-http --lib block::tests`
Expected: PASS。

Run: `cargo test -p wisp-fetcher --lib client::tests::fetch_http_blocks_configured_domain`
Expected: PASS。

Run: `cargo test -p wisp-fetcher --lib`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/http/src/block/blocker.rs crates/http/src/block/mod.rs crates/fetcher/src/client/fetch_client.rs crates/fetcher/src/client/mod.rs
git commit -m "fix: DomainBlocker 接入 HTTP 与浏览器抓取路径"
```

---

### Task 5: socks5 代理真实分流并删除 proxy 模块

**Files:**
- Modify: `crates/http/Cargo.toml`
- Modify: `crates/http/src/builder.rs`
- Modify: `crates/http/src/lib.rs`
- Delete: `crates/http/src/proxy.rs`
- Test: `crates/http/src/lib.rs`

**Interfaces:**
- Consumes: wreq `socks` feature。
- Produces: `ClientBuilder::build` 对 socks4/socks4a/socks5/socks5h 使用 `wreq::Proxy::http`，其余使用 `wreq::Proxy::all`；删除 `http::proxy`。

- [ ] **Step 1: 写失败测试**

`crates/http/src/lib.rs` 测试模块追加：

```rust
    #[test]
    fn socks5_proxy_builds() {
        let client = Client::builder()
            .proxy("socks5://127.0.0.1:1080")
            .build();
        assert!(client.is_ok(), "socks5 应可构建: {:?}", client.err());
    }

    #[test]
    fn socks4_proxy_builds() {
        let client = Client::builder()
            .proxy("socks4://127.0.0.1:1080")
            .build();
        assert!(client.is_ok(), "socks4 应可构建: {:?}", client.err());
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p wisp-http --lib tests::socks5_proxy_builds`
Expected: FAIL（wreq 未启用 `socks` feature 时 socks URI 构建失败）。

- [ ] **Step 3: 实现**

`crates/http/Cargo.toml`：

```toml
wreq = { version = "=6.0.0-rc.29", features = ["cookies", "stream", "socks"] }
```

`crates/http/src/builder.rs` 的 proxy 段替换：

```rust
        if let Some(ref proxy_url) = self.config.proxy {
            let proxy_scheme = url::Url::parse(proxy_url)
                .map(|u| u.scheme().to_string())
                .unwrap_or_default();
            let is_socks = matches!(
                proxy_scheme.as_str(),
                "socks4" | "socks4a" | "socks5" | "socks5h"
            );
            let proxy = if is_socks {
                wreq::Proxy::http(proxy_url)
            } else {
                wreq::Proxy::all(proxy_url)
            }
            .map_err(|e| {
                WispError::Network(NetworkError::Http(format!("proxy error: {e}")))
            })?;
            builder = builder.proxy(proxy);
        }
```

`crates/http/src/lib.rs` 删除：

```rust
pub mod proxy;
```

删除文件 `crates/http/src/proxy.rs`。

- [ ] **Step 4: 验证**

Run: `git rm crates/http/src/proxy.rs`

Run: `cargo test -p wisp-http --lib`
Expected: PASS。

Run: `rg -n "ParsedProxy|parse_proxies|to_proxy_url|http::proxy" crates src tests --glob "*.rs"`
Expected: 无输出。

- [ ] **Step 5: 提交**

```bash
git add crates/http/Cargo.toml crates/http/src/builder.rs crates/http/src/lib.rs Cargo.lock
git add -u crates/http/src/proxy.rs
git commit -m "feat: 按代理 scheme 分流支持 socks4/socks5，删除未使用的 proxy 解析模块"
```

---

### Task 6: 浏览器代理认证显式拒绝

**Files:**
- Modify: `crates/browser/src/launch/args.rs`
- Modify: `crates/browser/src/launch/tests.rs`
- Modify: `crates/browser/src/process.rs`
- Modify: `crates/browser/src/lib.rs`
- Modify: `crates/fetcher/src/client/fetch_client.rs`
- Modify: `crates/fetcher/src/client/mod.rs`

**Interfaces:**
- Changes: `push_proxy_arg` 返回 `Result<()>`；`build_stealth_args`/`build_default_args`/`process::build_chrome_args` 返回 `Result<Vec<String>>`；`FetchClient::build_browser_pool` 返回 `Result<Option<Arc<BrowserPool>>>`；代理含 userinfo 时返回 `WispError::Config("浏览器模式暂不支持代理认证")`。

- [ ] **Step 1: 写失败测试**

`crates/browser/src/launch/tests.rs` 把 `test_stealth_args_proxy_with_auth_still_sets_server` 替换为：

```rust
#[test]
fn test_stealth_args_proxy_with_auth_rejected() {
    let opts = LaunchOptions {
        proxy: Some(wisp_core::config::ProxyConfig {
            server: "http://127.0.0.1:8080".into(),
            username: Some("user".into()),
            password: Some("pass".into()),
        }),
        ..Default::default()
    };
    let err = build_stealth_args(&opts).expect_err("代理认证应被拒绝");
    assert!(err.to_string().contains("代理认证"), "错误应明确: {err}");
}
```

并把其余 `build_stealth_args(&opts)` / `build_default_args(&opts)` 改为 `.unwrap()`。

`crates/fetcher/src/client/mod.rs` 测试模块追加：

```rust
    #[cfg(feature = "browser")]
    #[test]
    fn browser_pool_rejects_authenticated_proxy() {
        let config = FetchClientConfig {
            proxy: Some("http://user:pass@127.0.0.1:8080".into()),
            ..Default::default()
        };
        let err = FetchClient::new(config).expect_err("认证代理应被拒绝");
        assert!(err.to_string().contains("代理认证"), "错误应明确: {err}");
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p wisp-browser --lib launch::tests`
Expected: FAIL（当前认证代理仍构建参数）。

Run: `cargo test -p wisp-fetcher --all-features --lib client::tests::browser_pool_rejects_authenticated_proxy`
Expected: FAIL（当前 `FetchClient::new` 成功）。

- [ ] **Step 3: 实现**

`crates/browser/src/launch/args.rs`：

```rust
//! Chrome 启动参数构建。

use wisp_core::config::LaunchOptions;
use wisp_core::error::{Result, WispError};

/// Build default Chrome launch arguments from options, with patches applied.
/// These args include the "--" prefix (for testing/verification).
pub fn build_default_args(options: &LaunchOptions) -> Result<Vec<String>> {
    Ok(build_stealth_args(options)?
        .iter()
        .map(|a| format!("--{a}"))
        .collect())
}

fn push_proxy_arg(args: &mut Vec<String>, options: &LaunchOptions) -> Result<()> {
    let Some(ref proxy) = options.proxy else {
        return Ok(());
    };
    if proxy.username.is_some() || proxy.password.is_some() {
        return Err(WispError::Config("浏览器模式暂不支持代理认证".into()));
    }
    args.push(format!("proxy-server={}", proxy.server));
    Ok(())
}

pub fn build_stealth_args(options: &LaunchOptions) -> Result<Vec<String>> {
    let mut args = base_stealth_args();
    push_proxy_arg(&mut args, options)?;
    push_extra_args(&mut args, options);
    Ok(args)
}
```

`crates/browser/src/process.rs`：

```rust
pub(super) fn build_chrome_args(
    options: &LaunchOptions,
    user_data_path: &std::path::Path,
) -> Result<Vec<String>> {
    let mut args = launch::build_stealth_args(options)?;
    // ...原有追加逻辑不变
    Ok(args.iter().map(|a| format!("--{a}")).collect())
}
```

`crates/browser/src/lib.rs` 的 `launch` 中：

```rust
        let chrome_args = process::build_chrome_args(&options, &user_data_path)?;
```

`crates/fetcher/src/client/fetch_client.rs` 的 `build_browser_pool`：

```rust
    #[cfg(feature = "browser")]
    fn build_browser_pool(config: &FetchClientConfig) -> Result<Option<Arc<BrowserPool>>> {
        if config.max_concurrent_pages == 0 {
            return Ok(None);
        }
        let proxy_config = match &config.proxy {
            Some(proxy) => {
                let parsed = url::Url::parse(proxy).map_err(|e| {
                    WispError::Config(format!("invalid proxy URL: {e}"))
                })?;
                if !parsed.username().is_empty() || parsed.password().is_some() {
                    return Err(WispError::Config(
                        "浏览器模式暂不支持代理认证".into(),
                    ));
                }
                Some(wisp_core::config::ProxyConfig {
                    server: proxy.clone(),
                    username: None,
                    password: None,
                })
            }
            None => None,
        };
        let launch_options = LaunchOptions {
            headless: config.headless,
            executable_path: config.executable_path.clone(),
            proxy: proxy_config,
            ..Default::default()
        };
        Ok(Some(BrowserPool::new(
            config.max_concurrent_pages,
            launch_options,
        )))
    }
```

`FetchClient::new` 中 `let browser_pool = Self::build_browser_pool(&config)?;`。

- [ ] **Step 4: 验证**

Run: `cargo test -p wisp-browser --lib launch::tests`
Expected: PASS。

Run: `cargo test -p wisp-fetcher --all-features --lib`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/browser/src/launch/args.rs crates/browser/src/launch/tests.rs crates/browser/src/process.rs crates/browser/src/lib.rs crates/fetcher/src/client/fetch_client.rs crates/fetcher/src/client/mod.rs
git commit -m "fix: 浏览器代理含认证时显式拒绝，不再静默无认证运行"
```

---

### Task 7: 移除无效 header_order 并校验 max_concurrent

**Files:**
- Modify: `crates/http/src/config.rs`
- Modify: `crates/http/src/builder.rs`
- Modify: `tests/fetch_test.rs`
- Modify: `crates/crawl/src/runner/builder/build.rs`
- Modify: `crates/crawl/src/runner/tests.rs`

**Interfaces:**
- Removes: `Config.header_order`、`ClientBuilder::header_order`。
- Produces: `EngineBuilder::build` 对 `max_concurrent == 0` 返回 `WispError::Config("max_concurrent must be > 0")`。

- [ ] **Step 1: 写失败测试**

`crates/crawl/src/runner/tests.rs` 追加：

```rust
#[test]
fn engine_builder_rejects_zero_max_concurrent() {
    let err = Engine::infra().max_concurrent(0).build().unwrap_err();
    assert!(
        err.to_string().contains("max_concurrent"),
        "错误应说明 max_concurrent: {err}"
    );
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p wisp-crawl --lib runner::tests::engine_builder_rejects_zero_max_concurrent`
Expected: FAIL（当前 build 成功）。

- [ ] **Step 3: 实现**

`crates/http/src/config.rs` 删除 `use wreq::header::HeaderName;`、`header_order` 字段及默认值。

`crates/http/src/builder.rs` 删除 `use wreq::header::HeaderName;`、`header_order` 方法、build 末尾的 header_order 注释。

`tests/fetch_test.rs` 删除 `use wreq::header::HeaderName;`、`test_builder_header_order`；`test_config_default_has_chrome136_emulation` 删除 `assert!(config.header_order.is_none());`；`test_builder_chain_emulation_and_header_order` 改为：

```rust
#[test]
fn test_builder_chain_emulation_and_timeout() {
    let builder = ClientBuilder::new()
        .emulation(Profile::Safari18)
        .timeout(std::time::Duration::from_secs(60))
        .user_agent("test-agent");
    let config = builder.config_ref();
    assert_eq!(config.emulation, Some(Profile::Safari18));
    assert_eq!(config.timeout, std::time::Duration::from_secs(60));
    assert_eq!(config.user_agent.as_deref(), Some("test-agent"));
}
```

`crates/crawl/src/runner/builder/build.rs` 的 `build()` 开头加入（Task 1 已加入则保留）：

```rust
        if self.max_concurrent == 0 {
            return Err(wisp_core::error::WispError::Config(
                "max_concurrent must be > 0".into(),
            ));
        }
```

- [ ] **Step 4: 验证**

Run: `cargo test -p wisp-crawl --lib runner::tests::engine_builder_rejects_zero_max_concurrent`
Expected: PASS。

Run: `cargo test --test fetch_test`
Expected: PASS。

Run: `rg -n "header_order" crates src tests --glob "*.rs"`
Expected: 无输出。

- [ ] **Step 5: 提交**

```bash
git add crates/http/src/config.rs crates/http/src/builder.rs tests/fetch_test.rs crates/crawl/src/runner/builder/build.rs crates/crawl/src/runner/tests.rs
git commit -m "fix: 移除无效 header_order 配置并校验 max_concurrent 必须大于 0"
```

---

### Task 8: FetchClient 优雅 Drop 与 Fetcher::from_client 补全

**Files:**
- Modify: `crates/fetcher/src/client/fetch_client.rs`
- Modify: `crates/fetcher/src/client/mod.rs`
- Modify: `crates/fetcher/src/fetcher.rs`
- Modify: `crates/fetcher/src/lib.rs`

**Interfaces:**
- Changes: `FetchClient` 实现 `Drop`（Runtime 内 spawn `BrowserPool::shutdown`）；`Fetcher::from_client` 返回 `Result<Self>` 并按配置自动构造 Dynamic/Stealth strategy；`Fetcher::new` 非 browser 分支删除重复 Auto 分支。

- [ ] **Step 1: 写失败测试**

`crates/fetcher/src/fetcher.rs` 追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{FetchClient, FetchClientConfig};
    use std::sync::Arc;

    #[test]
    fn from_client_http_works() {
        let client = Arc::new(
            FetchClient::new(FetchClientConfig {
                max_concurrent_pages: 0,
                ..Default::default()
            })
            .unwrap(),
        );
        let fetcher = Fetcher::from_client(client, FetchMode::Http).unwrap();
        assert_eq!(fetcher.mode(), FetchMode::Http);
    }

    #[cfg(feature = "browser")]
    #[test]
    fn from_client_dynamic_builds_strategy() {
        let client = Arc::new(FetchClient::new(FetchClientConfig::default()).unwrap());
        let fetcher = Fetcher::from_client(client, FetchMode::Dynamic).unwrap();
        assert!(
            fetcher.browser_strategy().is_some(),
            "from_client 应自动构造 Dynamic strategy"
        );
    }
}
```

`crates/fetcher/src/client/mod.rs` 追加：

```rust
    #[cfg(feature = "browser")]
    #[test]
    fn fetch_client_drop_does_not_panic_inside_runtime() {
        let config = FetchClientConfig::default();
        let client = FetchClient::new(config).expect("build client");
        drop(client);
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p wisp-fetcher --lib fetcher::tests::from_client_dynamic_builds_strategy`
Expected: FAIL（`from_client` 返回 `Self`，不能 `unwrap`）。

- [ ] **Step 3: 实现**

`crates/fetcher/src/client/fetch_client.rs` 追加：

```rust
#[cfg(feature = "browser")]
impl Drop for FetchClient {
    fn drop(&mut self) {
        let Some(pool) = self.browser_pool.take() else {
            return;
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let _ = handle.spawn(async move {
                pool.shutdown().await;
            });
        }
    }
}
```

`crates/fetcher/src/fetcher.rs`：

```rust
    /// 从已有 FetchClient 创建 Fetcher，自动构造 Dynamic/Stealth strategy。
    pub fn from_client(client: Arc<FetchClient>, mode: FetchMode) -> Result<Self> {
        if mode == FetchMode::Auto {
            return Err(WispError::Config(
                "Auto mode is owned by wisp_crawl Engine; use Http/Dynamic/Stealth explicitly"
                    .into(),
            ));
        }
        #[cfg(feature = "browser")]
        {
            let browser_strategy = Self::build_strategy(mode, client.config())?;
            Ok(Self {
                client,
                mode,
                browser_strategy,
            })
        }
        #[cfg(not(feature = "browser"))]
        {
            match mode {
                FetchMode::Http => Ok(Self { client, mode }),
                FetchMode::Dynamic | FetchMode::Stealth => Err(WispError::Config(format!(
                    "{mode:?} mode requires 'browser' feature"
                ))),
                FetchMode::Auto => unreachable!("Auto 已在 new/from_client 上方拒绝"),
            }
        }
    }
```

`Fetcher::new` 非 browser 分支：

```rust
        #[cfg(not(feature = "browser"))]
        {
            match mode {
                FetchMode::Http => Ok(Self { client, mode }),
                FetchMode::Dynamic | FetchMode::Stealth => Err(WispError::Config(format!(
                    "{mode:?} mode requires 'browser' feature"
                ))),
                FetchMode::Auto => unreachable!("Auto 已在函数开头拒绝"),
            }
        }
```

- [ ] **Step 4: 验证**

Run: `cargo test -p wisp-fetcher --lib`
Expected: PASS。

Run: `cargo test -p wisp-fetcher --all-features --lib`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/fetcher/src/client/fetch_client.rs crates/fetcher/src/client/mod.rs crates/fetcher/src/fetcher.rs crates/fetcher/src/lib.rs
git commit -m "fix: FetchClient Drop 优雅关闭浏览器池，from_client 自动构造 strategy"
```

---

### Task 9: checkpoint 统计全字段持久化

**Files:**
- Modify: `crates/crawl/src/observability/state.rs`
- Modify: `crates/crawl/src/observability/stats.rs`
- Modify: `crates/crawl/src/engine/checkpoint.rs`
- Modify: `tests/crawl_checkpoint_test.rs`
- Test: `crates/crawl/src/runner/tests.rs`

**Interfaces:**
- Changes: `CrawlState` 新增 `status_codes: HashMap<u16, usize>`、`blocked: usize`、`retries: usize`、`offsite: usize`、`cache_hits: usize`；删除 `CrawlState::from_stats`；`SpiderStats::restore_from` 恢复全部字段。

- [ ] **Step 1: 写失败测试**

`tests/crawl_checkpoint_test.rs` 的 `test_checkpoint_save_load_roundtrip` 改为：

```rust
#[tokio::test]
async fn test_checkpoint_save_load_roundtrip() {
    use std::collections::HashMap;

    let store = MemoryStore::default();

    let mut state = CrawlState::new("test-spider".to_string());
    state.pending_urls = vec![Request::get("https://example.com/pending")];
    state.pages_crawled = 42;
    state.items_scraped = 100;
    state.errors = 3;
    state.duration_ms = 5678;
    state.status_codes = HashMap::from([(200, 40), (404, 2)]);
    state.blocked = 1;
    state.retries = 2;
    state.offsite = 3;
    state.cache_hits = 4;

    let blob = bincode::serialize(&state).unwrap();
    wisp::storage::save_checkpoint(&store, "test-spider", &blob)
        .await
        .unwrap();

    let loaded = wisp::storage::load_checkpoint(&store, "test-spider")
        .await
        .unwrap()
        .expect("should be saved");
    let restored: CrawlState = bincode::deserialize(&loaded).unwrap();

    assert_eq!(restored.spider_name, "test-spider");
    assert_eq!(restored.pages_crawled, 42);
    assert_eq!(restored.items_scraped, 100);
    assert_eq!(restored.errors, 3);
    assert_eq!(restored.duration_ms, 5678);
    assert_eq!(restored.pending_urls.len(), 1);
    assert_eq!(restored.pending_urls[0].url, "https://example.com/pending");
    assert_eq!(restored.status_codes, HashMap::from([(200, 40), (404, 2)]));
    assert_eq!(restored.blocked, 1);
    assert_eq!(restored.retries, 2);
    assert_eq!(restored.offsite, 3);
    assert_eq!(restored.cache_hits, 4);

    let restored_stats = restored.to_stats();
    assert_eq!(restored_stats.pages_crawled, 42);
    assert_eq!(
        restored_stats.duration,
        std::time::Duration::from_millis(5678)
    );
    assert_eq!(restored_stats.status_code_counts, HashMap::from([(200, 40), (404, 2)]));
    assert_eq!(restored_stats.blocked_requests, 1);
    assert_eq!(restored_stats.retry_count, 2);
    assert_eq!(restored_stats.offsite_requests_count, 3);
    assert_eq!(restored_stats.cache_hits, 4);
}
```

`crates/crawl/src/runner/tests.rs` 追加：

```rust
#[test]
fn checkpoint_restores_full_stats() {
    use std::collections::HashMap;

    let mut state = CrawlState::new("s".into());
    state.status_codes = HashMap::from([(200, 10)]);
    state.blocked = 5;
    state.retries = 6;
    state.offsite = 7;
    state.cache_hits = 8;

    let stats = crate::SpiderStats::new();
    stats.restore_from(&state);
    assert_eq!(stats.blocked.load(std::sync::atomic::Ordering::SeqCst), 5);
    assert_eq!(stats.retries.load(std::sync::atomic::Ordering::SeqCst), 6);
    assert_eq!(stats.offsite.load(std::sync::atomic::Ordering::SeqCst), 7);
    assert_eq!(stats.cache_hits.load(std::sync::atomic::Ordering::SeqCst), 8);
    assert_eq!(stats.status_codes_snapshot(), HashMap::from([(200, 10)]));
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test crawl_checkpoint_test`
Expected: FAIL（`status_codes` 等字段不存在，或 `from_stats` 仍被引用）。

- [ ] **Step 3: 实现**

`crates/crawl/src/observability/state.rs`：

```rust
    /// 状态码分布。
    #[serde(default)]
    pub status_codes: HashMap<u16, usize>,
    /// 被拦截数。
    #[serde(default)]
    pub blocked: usize,
    /// 重试数。
    #[serde(default)]
    pub retries: usize,
    /// 站外请求数。
    #[serde(default)]
    pub offsite: usize,
    /// 缓存命中数。
    #[serde(default)]
    pub cache_hits: usize,
```

`CrawlState::new` 初始化：

```rust
            status_codes: HashMap::new(),
            blocked: 0,
            retries: 0,
            offsite: 0,
            cache_hits: 0,
```

删除 `CrawlState::from_stats`。

`CrawlState::to_stats`：

```rust
    pub fn to_stats(&self) -> CrawlStats {
        CrawlStats {
            items_scraped: self.items_scraped,
            pages_crawled: self.pages_crawled,
            errors: self.errors,
            duration: std::time::Duration::from_millis(self.duration_ms as u64),
            blocked_requests: self.blocked,
            retry_count: self.retries,
            status_code_counts: self.status_codes.clone(),
            offsite_requests_count: self.offsite,
            cache_hits: self.cache_hits,
        }
    }
```

`crates/crawl/src/observability/stats.rs` 的 `restore_from`：

```rust
    pub fn restore_from(&self, state: &CrawlState) {
        self.pages.store(state.pages_crawled, Ordering::SeqCst);
        self.items.store(state.items_scraped, Ordering::SeqCst);
        self.errors.store(state.errors, Ordering::SeqCst);
        self.blocked.store(state.blocked, Ordering::SeqCst);
        self.retries.store(state.retries, Ordering::SeqCst);
        self.offsite.store(state.offsite, Ordering::SeqCst);
        self.cache_hits.store(state.cache_hits, Ordering::SeqCst);
        self.elapsed_offset_ms
            .store(state.duration_ms as u64, Ordering::SeqCst);
        self.status_codes.clear();
        for (status, count) in &state.status_codes {
            self.status_codes
                .insert(*status, AtomicUsize::new(*count));
        }
        self.callback_pages.clear();
        for (callback, count) in &state.callback_pages {
            self.callback_pages.insert(callback.clone(), *count);
        }
    }
```

`crates/crawl/src/engine/checkpoint.rs`：

```rust
    let snapshot = snapshot_stats_for(stats, stats.status_codes_snapshot());
    let state = CrawlState {
        spider_name: spider_name.to_string(),
        pending_urls: pending,
        seen_urls: seen,
        items_scraped: snapshot.items_scraped,
        pages_crawled: snapshot.pages_crawled,
        errors: snapshot.errors,
        status_codes: snapshot.status_code_counts,
        blocked: snapshot.blocked_requests,
        retries: snapshot.retry_count,
        offsite: snapshot.offsite_requests_count,
        cache_hits: snapshot.cache_hits,
        callback_pages: stats.callback_pages_snapshot(),
        in_flight_urls: in_flight,
        duration_ms: snapshot.duration.as_millis(),
        saved_at: chrono::Utc::now(),
    };
```

同时删除文件头“`CrawlState::from_stats` 硬编码 seen_urls 为空”注释。

- [ ] **Step 4: 验证**

Run: `cargo test --test crawl_checkpoint_test`
Expected: PASS。

Run: `cargo test -p wisp-crawl --lib runner::tests::checkpoint_restores_full_stats`
Expected: PASS。

Run: `rg -n "from_stats" crates src tests --glob "*.rs"`
Expected: 无输出。

- [ ] **Step 5: 提交**

```bash
git add crates/crawl/src/observability/state.rs crates/crawl/src/observability/stats.rs crates/crawl/src/engine/checkpoint.rs crates/crawl/src/runner/tests.rs tests/crawl_checkpoint_test.rs
git commit -m "fix: checkpoint 统计补齐 status_codes/blocked/retries/offsite/cache_hits"
```

---

### Task 10: 无界增长上限

**Files:**
- Modify: `crates/crawl/src/auto/engine.rs`
- Modify: `crates/crawl/src/engine/fetch/page/auto_mode.rs`
- Test: 两个文件内测试模块

**Interfaces:**
- Produces: `ModeRuleEngine::learn` 在 auto_rules 达到 1000 后停止学习并告警一次；`cf_domain_locks` 达到 1024 后回退全局锁并告警一次；`Scheduler.seen` 行为不变。

- [ ] **Step 1: 写失败测试**

`crates/crawl/src/auto/engine.rs` 追加测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wisp_fetcher::FetchMode;

    fn unique_literal_url(i: usize) -> String {
        let mut n = i;
        let mut letters = String::new();
        loop {
            letters.push((b'a' + (n % 26) as u8) as char);
            n /= 26;
            if n == 0 {
                break;
            }
        }
        format!("https://example.com/{letters}")
    }

    #[test]
    fn auto_rules_stop_learning_at_limit() {
        let mut engine = ModeRuleEngine::new();
        for i in 0..AUTO_RULE_LIMIT {
            engine.learn(&unique_literal_url(i), FetchMode::Http);
        }
        assert_eq!(engine.auto_rule_count(), AUTO_RULE_LIMIT);
        engine.learn("https://example.com/never-learned", FetchMode::Stealth);
        assert_eq!(engine.auto_rule_count(), AUTO_RULE_LIMIT);
        assert_eq!(engine.resolve("https://example.com/never-learned"), None);
    }
}
```

`crates/crawl/src/engine/fetch/page/auto_mode.rs` 追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use dashmap::DashMap;

    #[test]
    fn domain_lock_inserts_below_limit() {
        let locks = DashMap::new();
        let lock = cf_domain_lock(&locks, "a.example.com");
        assert_eq!(locks.len(), 1);
        let _guard = lock.blocking_lock();
    }

    #[test]
    fn domain_lock_falls_back_at_limit() {
        let locks = DashMap::new();
        for i in 0..CF_DOMAIN_LOCK_LIMIT {
            locks.insert(format!("d{i}.example.com"), Arc::new(Mutex::new(())));
        }
        let lock = cf_domain_lock(&locks, "new.example.com");
        assert_eq!(locks.len(), CF_DOMAIN_LOCK_LIMIT);
        assert!(locks.get("new.example.com").is_none());
        let _guard = lock.blocking_lock();
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p wisp-crawl --lib auto::engine::tests::auto_rules_stop_learning_at_limit`
Expected: FAIL（`AUTO_RULE_LIMIT` 不存在）。

Run: `cargo test -p wisp-crawl --lib engine::fetch::page::auto_mode::tests::domain_lock_falls_back_at_limit`
Expected: FAIL（`CF_DOMAIN_LOCK_LIMIT`/`cf_domain_lock` 不存在）。

- [ ] **Step 3: 实现**

`crates/crawl/src/auto/engine.rs`：

```rust
const AUTO_RULE_LIMIT: usize = 1000;

pub struct ModeRuleEngine {
    user_rules: Vec<(Regex, FetchMode)>,
    auto_rules: Vec<(Regex, FetchMode)>,
    auto_rules_warned: bool,
}

// new() 中加 auto_rules_warned: false

    pub fn learn(&mut self, url: &str, mode: FetchMode) {
        let pattern = generalize_url(url);
        if let Ok(re) = Regex::new(&pattern) {
            for (existing_re, existing_mode) in &mut self.auto_rules {
                if existing_re.as_str() == re.as_str() {
                    *existing_mode = mode;
                    return;
                }
            }
            if self.auto_rules.len() >= AUTO_RULE_LIMIT {
                if !self.auto_rules_warned {
                    self.auto_rules_warned = true;
                    tracing::warn!("auto_rules 达到上限 {AUTO_RULE_LIMIT}，停止学习新规则");
                }
                return;
            }
            self.auto_rules.push((re, mode));
        }
    }
```

`crates/crawl/src/engine/fetch/page/auto_mode.rs`：

```rust
use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

const CF_DOMAIN_LOCK_LIMIT: usize = 1024;
static GLOBAL_CF_LOCK: LazyLock<Arc<Mutex<()>>> =
    LazyLock::new(|| Arc::new(Mutex::new(())));
static CF_LOCK_LIMIT_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn cf_domain_lock(
    locks: &dashmap::DashMap<String, Arc<Mutex<()>>>,
    domain: &str,
) -> Arc<Mutex<()>> {
    if locks.len() < CF_DOMAIN_LOCK_LIMIT {
        return locks
            .entry(domain.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
    }
    if !CF_LOCK_LIMIT_WARNED.swap(true, Ordering::SeqCst) {
        tracing::warn!("cf_domain_locks 达到上限 {CF_DOMAIN_LOCK_LIMIT}，回退全局锁");
    }
    GLOBAL_CF_LOCK.clone()
}
```

`fetch_stealth_override` 中替换：

```rust
    let lock = cf_domain_lock(cf_domain_locks, &domain);
```

- [ ] **Step 4: 验证**

Run: `cargo test -p wisp-crawl --lib auto::engine::tests`
Expected: PASS。

Run: `cargo test -p wisp-crawl --lib engine::fetch::page::auto_mode::tests`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/crawl/src/auto/engine.rs crates/crawl/src/engine/fetch/page/auto_mode.rs
git commit -m "fix: auto_rules 与 cf_domain_locks 增加容量上限，超限显式降级"
```

---

### Task 11: 死代码与命名清理

**Files:**
- Delete: `crates/http/src/ua.rs`
- Modify: `crates/http/src/lib.rs`
- Modify: `src/lib.rs`
- Modify: `README.md`
- Modify: `crates/browser/src/lib.rs`
- Modify: `crates/fetcher/src/strategy/extract.rs`
- Modify: `crates/fetcher/src/strategy/event.rs`
- Rename: `crates/crawl/src/middleware/builtin/limit.rs` → `crates/crawl/src/middleware/builtin/cache_robots_delay.rs`
- Modify: `crates/crawl/src/middleware/builtin.rs`

**Interfaces:** 无公共 API 新增；删除 `UaRotator`、`Browser.user_data_path` 字段与过时 allow。

- [ ] **Step 1: 实现**

删除 `crates/http/src/ua.rs`：

```bash
git rm crates/http/src/ua.rs
```

`crates/http/src/lib.rs` 删除：

```rust
pub mod ua;
pub use ua::UaRotator;
```

`src/lib.rs` 删除：

```rust
pub use http::UaRotator;
```

`README.md` 的 `crates/http/` 行改为：

```text
crates/http/                     wreq Client/Config/DomainBlocker
```

`crates/browser/src/lib.rs`：

```rust
pub struct Browser {
    session: Arc<CdpSession>,
    process: tokio::process::Child,
    /// 自动清理守卫：Drop 时自动删除临时目录。
    /// 用户自定义 user_data_dir 时为 None（不清理）。
    _temp_dir: Option<tempfile::TempDir>,
    headless: bool,
}
```

并在 `launch` 返回值中删除 `user_data_path,` 字段。

`crates/fetcher/src/strategy/extract.rs` 删除：

```rust
#[allow(dead_code)] // PR2 后续 task 将由 FetchClient 接入
```

`crates/fetcher/src/strategy/event.rs` 删除：

```rust
#[allow(dead_code)] // PR2 后续 task 将由 FetchClient 接入
```

重命名：

```bash
git mv crates/crawl/src/middleware/builtin/limit.rs crates/crawl/src/middleware/builtin/cache_robots_delay.rs
```

`crates/crawl/src/middleware/builtin.rs`：

```rust
mod cache_robots_delay;
// ...
pub use cache_robots_delay::{CacheMiddleware, DelayMiddleware, RobotsMiddleware};
```

- [ ] **Step 2: 验证**

Run: `rg -n "UaRotator|user_data_path|mod limit|limit::" crates src tests --glob "*.rs"`
Expected: 无输出。

Run: `cargo check --workspace --all-features --all-targets`
Expected: PASS。

- [ ] **Step 3: 提交**

```bash
git add crates/http/src/ua.rs crates/http/src/lib.rs src/lib.rs README.md crates/browser/src/lib.rs crates/fetcher/src/strategy/extract.rs crates/fetcher/src/strategy/event.rs
git add -A crates/crawl/src/middleware/builtin
git commit -m "refactor: 删除 UaRotator/死字段/过时标注并重命名 limit 模块"
```

---

### Task 12: 文档同步与全量验证

**Files:**
- Modify: `docs/known-issues.md`

- [ ] **Step 1: 更新完成记录**

`docs/known-issues.md` 的完成记录区追加：

```markdown
- 半成品与冗余修复已完成：MCP stealth_fetch 复用共享 FetchClient/StealthStrategy；
  crawl_site 真实跟随链接；adaptive_scrape db_path 生效；MCP --db 无 sqlite 时明确降级；
  DomainBlocker 接入 HTTP/浏览器；socks4/socks5 真实分流；浏览器代理认证显式拒绝；
  移除无效 header_order；max_concurrent(0) 构建报错；FetchClient Drop 优雅关闭浏览器池；
  from_client 自动构造 strategy；checkpoint 统计全字段恢复；auto_rules/cf_domain_locks 有界；
  清理 UaRotator/from_stats/user_data_path/过时标注。
```

- [ ] **Step 2: 全量验证**

Run:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-features --all-targets
cargo check --workspace --no-default-features --all-targets
cargo clippy --all-targets --all-features --message-format=short
cargo test --workspace --all-features --lib --no-fail-fast
cargo test --workspace --all-features --tests --no-run --no-fail-fast
```

Expected: 全部 PASS；Clippy 无 error；无 `header_order`/`UaRotator`/`ParsedProxy`/`from_stats`/`user_data_path` 残留引用。

- [ ] **Step 3: 提交**

```bash
git add docs/known-issues.md
git commit -m "docs: 记录半成品与冗余修复完成项"
```

---

## Self-Review

- Spec 覆盖：18 项问题分别落在 Task 1（MCP stealth/共享）、Task 2（crawl_site）、Task 3（adaptive/db）、Task 4（DomainBlocker）、Task 5（socks）、Task 6（浏览器代理认证）、Task 7（header_order/max_concurrent）、Task 8（Drop/from_client）、Task 9（checkpoint 统计）、Task 10（无界增长）、Task 11（死代码/命名）、Task 12（文档/验证）。
- 类型一致性：`EngineBuilder::fetch_client` 在 Task 1 定义，Task 1/12 使用；`DomainBlocker::blocked_domains` 在 Task 4 定义；`CrawlState` 新字段在 Task 9 定义；所有后续任务只消费本计划已定义签名。
- 未纳入：`validate_url` 对 localhost/DNS rebinding 的防护保留为已知局限；`Browser::Drop` 同步 sleep 仅作观察项。
