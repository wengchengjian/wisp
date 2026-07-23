# FetchClient 统一重构计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 统一 Fetcher 和 Engine 的底层请求客户端，消除重复代码，通过 BrowserPool 解决浏览器泄漏问题。

**Architecture:** 创建 `FetchClient` 结构体封装 `Arc<http::Client>` + `Option<Arc<BrowserPool>>`，作为所有请求的统一入口。Fetcher 简化为 `FetchClient` 的薄包装，Engine 直接使用 `FetchClient`。删除 `fetcher::Session`、`http::HttpSession` 等重复抽象。

**Tech Stack:** Rust, tokio, aho-corasick, moka, CDP

## Global Constraints

- 从不向后兼容，只考虑最优解
- 变量命名 snake_case
- 提交信息用中文，一行
- 只在 master 分支开发
- TDD：先写测试再写实现
- 完成后运行全部测试验证

## 当前问题

```
Fetcher（一次性请求）                Engine（持续爬取）
├── http::Client (每次新建!)          ├── http::Client (共享 ✅)
├── Browser (每次新建,泄漏!)          ├── Fetcher::dynamic() → Browser (每次新建,泄漏!)
├── fetcher::Session (cookie jar)    ├── 无 Session
├── FetcherConfig                    ├── EngineConfig (含 fetcher_config)
└── Request/Response                 └── SpiderRequest/SpiderResponse
```

## 目标架构

```
FetchClient（统一请求客户端）
├── http: Arc<http::Client>              // 共享 HTTP 客户端
├── browser_pool: Option<Arc<BrowserPool>>  // 可选浏览器池
└── config: FetchClientConfig             // 统一配置

Fetcher（薄包装，一次性请求场景）
├── client: Arc<FetchClient>
└── mode: FetchMode

Engine（使用 FetchClient）
├── client: Arc<FetchClient>
└── ...（其他引擎状态）
```

## 文件结构

| 文件 | 职责 | 操作 |
|------|------|------|
| `src/fetcher/client.rs` | `FetchClient` 结构体 + `FetchClientConfig` | 新建 |
| `src/fetcher/mod.rs` | `Fetcher`（薄包装） + `FetchMode` | 修改 |
| `src/fetcher/session.rs` | 删除 | 删除 |
| `src/fetcher/response.rs` | `Request`/`Response`/`Method` | 保留 |
| `src/http/session.rs` | 删除 | 删除 |
| `src/http/mod.rs` | `Client`/`Config` | 保留（底层不变） |
| `src/browser/pool.rs` | `BrowserPool` | 保留（已有 RAII） |
| `src/crawl/engine.rs` | `fetch_page_inner` 改用 `FetchClient` | 修改 |
| `src/crawl/mod.rs` | `EngineConfig` 调整 | 修改 |
| `src/crawl/runner.rs` | 构造 `FetchClient` 传给 Engine | 修改 |
| `src/lib.rs` | 公共 API 导出更新 | 修改 |

---

## Task 1: 创建 FetchClient

**Files:**
- Create: `src/fetcher/client.rs`
- Modify: `src/fetcher/mod.rs`（添加 `pub mod client;`）

**Interfaces:**
- Produces: `FetchClient`、`FetchClientConfig`、`FetchClientBuilder`

**设计：**

```rust
// src/fetcher/client.rs

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;

use crate::error::{Result, WispError};
use crate::http::{Client, Config as HttpConfig};
use crate::browser::{BrowserPool, pool::BrowserHandle};
use crate::config::LaunchOptions;
use crate::stealth::challenge::ChallengeSolver;
use crate::stealth::human::HumanBehavior;

use super::response::{Request, Response, Method};

/// 统一请求客户端配置。
#[derive(Debug, Clone)]
pub struct FetchClientConfig {
    pub timeout: Duration,
    pub max_redirects: usize,
    pub proxy: Option<String>,
    pub user_agent: Option<String>,
    pub headers: HashMap<String, String>,
    // Browser 相关
    pub headless: bool,
    pub human_mode: bool,
    pub challenge_timeout: Duration,
    pub wait_for: Option<String>,
    pub extra_wait_ms: u64,
    // BrowserPool 相关
    pub browser_pool_size: usize,
    pub browser_idle_timeout: Duration,
}

impl Default for FetchClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_redirects: 10,
            proxy: None,
            user_agent: None,
            headers: HashMap::new(),
            headless: true,
            human_mode: true,
            challenge_timeout: Duration::from_secs(30),
            wait_for: None,
            extra_wait_ms: 0,
            browser_pool_size: 2,
            browser_idle_timeout: Duration::from_secs(300),
        }
    }
}

/// 统一请求客户端：封装 HTTP Client 和 BrowserPool。
///
/// - HTTP 请求：共享 `http::Client`（连接池复用）
/// - 浏览器请求：通过 `BrowserPool`（实例复用，RAII 自动归还）
pub struct FetchClient {
    http: Arc<Client>,
    browser_pool: Option<Arc<BrowserPool>>,
    config: FetchClientConfig,
}

impl FetchClient {
    /// 创建 FetchClient。
    pub fn new(config: FetchClientConfig) -> Result<Self> {
        let http = Arc::new(Self::build_http_client(&config)?);
        let browser_pool = Self::build_browser_pool(&config);
        Ok(Self { http, browser_pool, config })
    }

    /// 获取 HTTP 客户端引用。
    pub fn http(&self) -> &Client { &self.http }

    /// 获取浏览器池引用（若有）。
    pub fn browser_pool(&self) -> Option<&Arc<BrowserPool>> { self.browser_pool.as_ref() }

    /// 获取配置引用。
    pub fn config(&self) -> &FetchClientConfig { &self.config }

    /// HTTP 请求（共享 Client）。
    pub async fn fetch_http(&self, req: &Request) -> Result<Response> {
        let extra_headers: Vec<(String, String)> = req.headers.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let resp = match req.method {
            Method::Get => self.http.get(&req.url, &extra_headers).await?,
            Method::Post => self.http.post(&req.url, req.body.as_deref(), None, &extra_headers).await?,
            Method::Put => self.http.put(&req.url, req.body.as_deref(), None, &extra_headers).await?,
            Method::Delete => self.http.delete(&req.url, &extra_headers).await?,
        };
        Ok(Response::from_http(
            resp.status, resp.url.clone(), resp.headers.clone(), resp.body.clone(),
            resp.headers.get("content-type").cloned().unwrap_or_default(),
            Some(req.clone()),
        ))
    }

    /// 浏览器请求（通过 BrowserPool，实例复用）。
    /// solve_cf=true 时执行 CF 挑战解决 + 人类行为模拟。
    pub async fn fetch_browser(&self, req: &Request, solve_cf: bool) -> Result<Response> {
        let pool = self.browser_pool.as_ref()
            .ok_or_else(|| WispError::CdpError("browser pool not configured".into()))?;
        let handle = pool.acquire().await?;
        let browser = pool.get_browser(handle.index).await
            .ok_or_else(|| WispError::CdpError("browser instance lost".into()))?;
        // RAII: handle 在函数结束时 Drop，自动归还到池
        let result = self.do_browser_work(&browser, req, solve_cf).await;
        drop(handle); // 显式归还（实际上 Drop 也会做）
        result
    }

    /// 浏览器工作逻辑（从 fetcher/mod.rs 迁移）。
    async fn do_browser_work(
        &self,
        browser: &crate::browser::pool::BrowserRef,
        req: &Request,
        solve_cf: bool,
    ) -> Result<Response> {
        let mut page = browser.new_page().await?;
        let _ = page.cmd("Network.enable", serde_json::json!({})).await;
        page.goto(&req.url).await?;
        let nav_status = self.capture_navigation_status(&page).await;
        if solve_cf {
            let solver = ChallengeSolver::new(&page);
            solver.solve(self.config.challenge_timeout).await?;
            if self.config.human_mode {
                let human = HumanBehavior::new(&page);
                human.random_delay(500, 1500).await?;
                human.random_scroll().await?;
                human.random_delay(300, 800).await?;
            }
        }
        if let Some(ref selector) = self.config.wait_for {
            page.wait_for_selector(selector, self.config.timeout.as_millis() as u64).await?;
        }
        if self.config.extra_wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.config.extra_wait_ms)).await;
        }
        self.extract_browser_response(&page, req, nav_status).await
    }

    // ... capture_navigation_status / extract_browser_response 从 fetcher/mod.rs 迁移 ...

    fn build_http_client(config: &FetchClientConfig) -> Result<Client> {
        let mut builder = Client::builder()
            .timeout(config.timeout)
            .max_redirects(config.max_redirects);
        if let Some(ref proxy) = config.proxy {
            builder = builder.proxy(proxy);
        }
        if let Some(ref ua) = config.user_agent {
            builder = builder.user_agent(ua);
        }
        for (k, v) in &config.headers {
            builder = builder.header(k, v);
        }
        builder.build()
    }

    fn build_browser_pool(config: &FetchClientConfig) -> Option<Arc<BrowserPool>> {
        if config.browser_pool_size == 0 {
            return None;
        }
        let proxy_config = config.proxy.as_ref().map(|p| crate::config::ProxyConfig {
            server: p.clone(), username: None, password: None,
        });
        let launch_options = LaunchOptions {
            headless: config.headless,
            proxy: proxy_config,
            ..Default::default()
        };
        Some(BrowserPool::new(
            config.browser_pool_size,
            config.browser_idle_timeout,
            launch_options,
        ))
    }
}
```

- [ ] **Step 1:** 创建 `src/fetcher/client.rs`，实现 `FetchClient` + `FetchClientConfig`
- [ ] **Step 2:** 在 `src/fetcher/mod.rs` 添加 `pub mod client;` + `pub use client::{FetchClient, FetchClientConfig};`
- [ ] **Step 3:** 迁移 `capture_navigation_status` 和 `extract_browser_response` 从 `fetcher/mod.rs` 到 `FetchClient`
- [ ] **Step 4:** 写单元测试（HTTP 请求 + 配置构建）
- [ ] **Step 5:** `cargo test --lib fetcher::client` 验证通过
- [ ] **Step 6:** 提交

---

## Task 2: Fetcher 委托给 FetchClient

**Files:**
- Modify: `src/fetcher/mod.rs`（Fetcher 改为薄包装）
- Delete: `src/fetcher/session.rs`

**设计：**

```rust
// src/fetcher/mod.rs（简化后）

pub struct Fetcher {
    client: Arc<FetchClient>,
    mode: FetchMode,
}

impl Fetcher {
    pub fn new(mode: FetchMode, config: FetchClientConfig) -> Result<Self> {
        Ok(Self { client: Arc::new(FetchClient::new(config)?), mode })
    }

    pub async fn fetch(&self, req: Request) -> Result<Response> {
        match self.mode {
            FetchMode::Http | FetchMode::Auto => self.client.fetch_http(&req).await,
            FetchMode::Dynamic => self.client.fetch_browser(&req, false).await,
            FetchMode::Stealth => self.client.fetch_browser(&req, true).await,
        }
    }
    // ... get/post 等便捷方法委托给 fetch ...
}
```

**删除：**
- `Fetcher::fetch_http`（每次新建 Client 的逻辑）
- `Fetcher::launch_browser`（每次新建 Browser 的逻辑）
- `Fetcher::fetch_browser_page`（迁移到 FetchClient）
- `fetcher::Session`（整个文件删除）

- [ ] **Step 1:** 重写 `Fetcher` 为 `FetchClient` 的薄包装
- [ ] **Step 2:** 删除 `src/fetcher/session.rs`
- [ ] **Step 3:** 更新 `src/fetcher/mod.rs` 的 `pub mod` 声明
- [ ] **Step 4:** `cargo build` 验证编译通过
- [ ] **Step 5:** 提交

---

## Task 3: Engine 使用 FetchClient

**Files:**
- Modify: `src/crawl/engine.rs`（`fetch_page_inner` 改用 `FetchClient`）
- Modify: `src/crawl/mod.rs`（`EngineConfig` 调整）
- Modify: `src/crawl/runner.rs`（构造 `FetchClient`）

**核心改动：**

`EngineConfig` 中 `fetcher_config: http::Config` 改为 `fetch_client: Arc<FetchClient>`：

```rust
pub struct EngineConfig {
    pub client: Arc<FetchClient>,  // 替代原来的 client + fetcher_config
    pub fetch_mode: FetchMode,
    // ... 其他字段不变 ...
}
```

`fetch_page_inner` 中浏览器模式直接用 `FetchClient`：

```rust
// 之前：Fetcher::dynamic() / Fetcher::stealth()（每次新建 Browser）
// 之后：
if mode == FetchMode::Dynamic || mode == FetchMode::Stealth {
    let solve_cf = mode == FetchMode::Stealth;
    let req = Request { url: req.url.clone(), method: req.method.into(), ..Default::default() };
    let resp = ctx.config.client.fetch_browser(&req, solve_cf).await?;
    return Ok(SpiderResponse { ... });
}
```

- [ ] **Step 1:** 修改 `EngineConfig`：`client: Arc<FetchClient>` + 删除 `fetcher_config`
- [ ] **Step 2:** 修改 `fetch_page_inner`：浏览器模式用 `ctx.config.client.fetch_browser()`
- [ ] **Step 3:** 修改 `runner.rs`：构造 `FetchClient` 传给 `EngineConfig`
- [ ] **Step 4:** 修改 `engine.rs` 的 HTTP 模式：用 `ctx.config.client.http()` 获取共享 Client
- [ ] **Step 5:** `cargo build` 验证编译通过
- [ ] **Step 6:** `cargo test --lib` 验证测试通过
- [ ] **Step 7:** 提交

---

## Task 4: 删除冗余代码

**Files:**
- Delete: `src/http/session.rs`（`HttpSession`）
- Modify: `src/http/mod.rs`（删除 `pub mod session;`）
- Delete: `src/crawl/request_cache.rs`（旧版，只有 url 键）
- Modify: `src/crawl/mod.rs`（删除旧版导出）

- [ ] **Step 1:** 删除 `src/http/session.rs`
- [ ] **Step 2:** 删除 `src/crawl/request_cache.rs`
- [ ] **Step 3:** 更新 `src/http/mod.rs` 和 `src/crawl/mod.rs` 的 mod 声明
- [ ] **Step 4:** `cargo build` 验证编译通过
- [ ] **Step 5:** 提交

---

## Task 5: 更新公共 API 和测试

**Files:**
- Modify: `src/lib.rs`（导出更新）
- Modify: `tests/`（更新所有使用旧 API 的测试）

**导出更新：**
```rust
// src/lib.rs
pub use fetcher::{Fetcher, FetchMode, FetcherConfig, FetcherBuilder};
pub use fetcher::{Response, Request, Method};
pub use fetcher::client::{FetchClient, FetchClientConfig};  // 新增
// 删除：pub use fetcher::Session;
// 删除：pub use http::HttpSession;
```

- [ ] **Step 1:** 更新 `src/lib.rs` 导出
- [ ] **Step 2:** 更新所有测试文件中的旧 API 引用
- [ ] **Step 3:** `cargo test` 全量验证
- [ ] **Step 4:** `cargo clippy` 检查
- [ ] **Step 5:** 提交

---

## Self-Review

**1. Spec coverage:**
- ✅ 统一请求客户端 → Task 1 (FetchClient)
- ✅ 删除冗余代码 → Task 4 (Session、旧 RequestCache)
- ✅ 解决浏览器泄漏 → Task 1 (BrowserPool RAII)
- ✅ Engine 使用统一客户端 → Task 3

**2. Placeholder scan:**
- `capture_navigation_status` 和 `extract_browser_response` 的迁移在 Task 1 Step 3 中标注，需要从 `fetcher/mod.rs` 原样迁移
- `do_browser_work` 是从 `fetch_browser_page` 迁移的逻辑

**3. Type consistency:**
- `FetchClient` 在 Task 1 定义，Task 2 和 Task 3 使用
- `FetchClientConfig` 在 Task 1 定义，Task 3 替代 `FetcherConfig` 和 `http::Config`
- `Request`/`Response` 类型保持不变（在 `fetcher/response.rs`）
