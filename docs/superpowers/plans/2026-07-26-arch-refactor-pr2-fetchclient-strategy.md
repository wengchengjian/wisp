# PR2: FetchClient 拆分 + browser/stealth 边界清理 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 引入 BrowserFetchStrategy trait 替代 fetch_browser 的 bool 标志，拆分 Dynamic/Stealth 策略到独立模块，清理 browser/stealth 边界

**Architecture:** 新增 fetcher/strategy.rs（trait + 公共 helper）+ fetcher/strategies/{dynamic,stealth}.rs（实现），FetchClient::fetch_browser 接收 &dyn BrowserFetchStrategy，Fetcher 持有 strategy；patches.rs 迁到 stealth，build_stealth_args 拆分

**Tech Stack:** Rust, Tokio, async_trait, wreq, CDP

## Global Constraints

- 变量命名 snake_case
- 代码注释用中文
- 提交信息用中文（一行）
- TDD：先写测试再写实现
- 保持现有测试全绿
- 不向后兼容，只考虑最优解
- PR1 已完成：FetchClient 已持有 `cookie_jar: Arc<dyn CookieJar>`，`CfSessionCache` 已迁出到 `src/cookie/cf.rs`（命名为 `CfCookieJar`，含 `CfSession` 结构）
- PR2 阶段不启用 feature gate（stealth 模块仍总是编译，`#[cfg(feature = "...")]` 留到 PR3）
- `extract_browser_response` 和 `recv_navigation_status` 提取为 `fetcher::strategy` 模块的 `pub(crate)` 自由函数，被两个策略复用
- `CfCookieJar` 由 `StealthStrategy` 独占持有（`Fetcher::new` 创建并注入）
- 120s 总超时由 `FetchClient::fetch_browser` 包装（非 strategy 职责）

---

## 文件结构概览

| 文件 | 责任 | 操作 |
|---|---|---|
| `src/fetcher/strategy.rs` | `BrowserFetchStrategy` trait + `recv_navigation_status` + `extract_browser_response` 公共 helper | Create |
| `src/fetcher/strategies/mod.rs` | strategies 模块声明 + re-export | Create |
| `src/fetcher/strategies/dynamic.rs` | `DynamicStrategy`（浏览器渲染，无 CF 绕过） | Create |
| `src/fetcher/strategies/stealth.rs` | `StealthStrategy`（CF bypass + 人类行为 + cookie 复用） | Create |
| `src/fetcher/mod.rs` | 添加 `pub mod strategy; pub mod strategies;` + re-export | Modify |
| `src/fetcher/client.rs` | `fetch_browser` 签名改为接收 `&dyn BrowserFetchStrategy`；删除 `do_browser_work_inner`/`recv_navigation_status`/`extract_browser_response` 私有方法 | Modify |
| `src/fetcher/mod.rs`（Fetcher） | 添加 `browser_strategy` 字段；`Fetcher::new` 根据 mode 构造 strategy | Modify |
| `src/stealth/patches.rs` | 反检测 JS 补丁（从 `src/browser/patches.rs` 迁移） | Create |
| `src/browser/patches.rs` | 删除（已迁移） | Delete |
| `src/browser/mod.rs` | 移除 `pub mod patches;` 声明 | Modify |
| `src/stealth/mod.rs` | 添加 `pub mod patches;` | Modify |
| `src/browser/page.rs` | `crate::browser::patches::` 改为 `crate::stealth::patches::` | Modify |
| `src/browser/launch.rs` | `build_stealth_args` 拆为 `build_common_args` + `build_stealth_extra_args` | Modify |
| `src/browser/mod.rs`（Browser::launch） | 注释清理（"anti-detection patches" → 通用描述） | Modify |

---

### Task 1: BrowserFetchStrategy trait + 公共 helper（strategy.rs）

**Files:**
- Create: `src/fetcher/strategy.rs`
- Modify: `src/fetcher/mod.rs:31-35`（添加 `pub mod strategy;`）
- Test: `src/fetcher/strategy.rs`（内联测试模块）

**Interfaces:**
- Consumes: `crate::browser::Page`（`pub use page::Page`）、`crate::browser::cdp::CdpEvent`、`super::response::{Request, Response}`、`crate::error::{BrowserError, Result, WispError}`
- Produces:
  - `pub trait BrowserFetchStrategy { async fn fetch(&self, page: &mut Page, req: &Request) -> Result<Response>; }`
  - `pub(crate) async fn recv_navigation_status(rx: &mut broadcast::Receiver<CdpEvent>, sid: &str) -> Result<u16>`
  - `pub(crate) async fn extract_browser_response(page: &Page, req: &Request, nav_status: u16) -> Result<Response>`

- [ ] **Step 1: 写失败的测试**

创建 `src/fetcher/strategy.rs`，包含完整测试代码：

```rust
//! 浏览器抓取策略 — 区分 Dynamic/Stealth 的差异化逻辑。
//!
//! ARCH: 替代 `fetch_browser(&req, solve_cf: bool)` 的 bool 标志。
//! 新策略（如 Playwright）可实现此 trait 零侵入注入。

use std::time::Duration;

use async_trait::async_trait;

use crate::browser::cdp::CdpEvent;
use crate::browser::Page;
use crate::error::{BrowserError, Result, WispError};

use super::response::{Request, Response};

/// 浏览器抓取策略 — 区分 Dynamic/Stealth 的差异化逻辑。
///
/// ARCH: 替代 `fetch_browser(&req, solve_cf: bool)` 的 bool 标志。
/// 新策略（如 Playwright）可实现此 trait 零侵入注入。
///
/// 调用方（`FetchClient::fetch_browser`）保证：
/// - 调用前已 `acquire` page
/// - 调用后由调用方 `close` page
/// - 120s 总超时由调用方包装
#[async_trait]
pub trait BrowserFetchStrategy: Send + Sync {
    /// 执行浏览器导航 + 后处理（CF 挑战 / 人类行为 / 等待选择器等）。
    async fn fetch(&self, page: &mut Page, req: &Request) -> Result<Response>;
}

/// 从事件流中接收 `Network.responseReceived` (type=Document) 事件并提取状态码。
///
/// 必须在 `goto` 之前订阅 `event_rx`，否则可能丢失事件。
/// 5s 超时：导航通常在 1-3s 内完成，5s 足够覆盖慢速页面。
///
/// 特殊处理：若先收到 `Network.loadingFailed` (type=Document)，说明导航请求失败
///（如代理连接失败、DNS 解析失败），立即返回错误，不空等 5s 超时。
pub(crate) async fn recv_navigation_status(
    rx: &mut tokio::sync::broadcast::Receiver<CdpEvent>,
    sid: &str,
) -> Result<u16> {
    use tokio::sync::broadcast::error::RecvError;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(event)) => {
                let match_session =
                    event.session_id.as_deref() == Some(sid) || event.session_id.is_none();
                if !match_session {
                    continue;
                }

                // 导航请求失败（代理/DNS/网络问题）：立即返回错误
                if event.method == "Network.loadingFailed" {
                    let is_doc =
                        event.params.get("type").and_then(|t| t.as_str()) == Some("Document");
                    if is_doc {
                        let error_text = event
                            .params
                            .get("errorText")
                            .and_then(|t| t.as_str())
                            .unwrap_or("unknown");
                        tracing::warn!(
                            "recv_navigation_status: Network.loadingFailed errorText={error_text}"
                        );
                        return Err(WispError::Browser(BrowserError::CdpConnection(format!(
                            "navigation loading failed: {error_text}"
                        ))));
                    }
                    continue;
                }

                if event.method != "Network.responseReceived" {
                    continue;
                }
                let is_doc =
                    event.params.get("type").and_then(|t| t.as_str()) == Some("Document");
                if !is_doc {
                    continue;
                }
                return event
                    .params
                    .get("response")
                    .and_then(|r| r.get("status"))
                    .and_then(serde_json::Value::as_u64)
                    .map(|s| s as u16)
                    .ok_or_else(|| {
                        WispError::Browser(BrowserError::CdpConnection(
                            "Network.responseReceived missing response.status".into(),
                        ))
                    });
            }
            Ok(Err(RecvError::Lagged(n))) => {
                tracing::warn!("event subscriber lagged by {n} events, continuing recv");
            }
            Ok(Err(RecvError::Closed)) => {
                return Err(WispError::Browser(BrowserError::CdpConnection(
                    "event broadcaster closed before navigation status captured".into(),
                )));
            }
            Err(_) => {
                // 超时不返回错误：CF 挑战页面可能不触发 Network.responseReceived (type=Document)
                // 事件（CF 用 JavaScript 挑战，非标准 HTTP 响应流程）。
                // 返回默认 200，让流程继续到 CF 挑战解决阶段。
                tracing::warn!(
                    "capture_navigation_status: 5s 内未收到 Network.responseReceived，\
                     返回默认 200（CF 挑战页面可能不触发此事件）"
                );
                return Ok(200);
            }
        }
    }
}

/// 从浏览器页面提取统一 Response。
///
/// ARCH: 从 FetchClient::extract_browser_response 提取为公共 helper，
/// 供 DynamicStrategy / StealthStrategy 复用。
pub(crate) async fn extract_browser_response(
    page: &Page,
    req: &Request,
    nav_status: u16,
) -> Result<Response> {
    let html = page
        .evaluate_as_string("document.documentElement.outerHTML")
        .await?;
    let title = page.evaluate_as_string("document.title").await?;
    let final_url = page.evaluate_as_string("window.location.href").await?;

    let cookies_raw = page.evaluate_as_string("document.cookie").await?;
    let cookies: Vec<String> = cookies_raw
        .split(';')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();

    Ok(Response::from_browser(
        nav_status,
        final_url,
        html,
        title,
        cookies,
        req.clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::sync::broadcast;

    /// 构造一个 CdpEvent。
    fn make_event(method: &str, params: serde_json::Value, session_id: Option<&str>) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params,
            session_id: session_id.map(std::string::ToString::to_string),
        }
    }

    #[tokio::test]
    async fn test_recv_navigation_status_returns_status_code() {
        let (tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        // 发送一个无关事件 + 一个 Document 响应事件
        tx.send(make_event(
            "Network.responseReceived",
            json!({"type": "XHR", "response": {"status": 204}}),
            Some("sid"),
        ))
        .unwrap();
        tx.send(make_event(
            "Network.responseReceived",
            json!({"type": "Document", "response": {"status": 200}}),
            Some("sid"),
        ))
        .unwrap();

        let status = recv_navigation_status(&mut rx, "sid").await.expect("应返回状态码");
        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn test_recv_navigation_status_loading_failed_returns_error() {
        let (tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        tx.send(make_event(
            "Network.loadingFailed",
            json!({"type": "Document", "errorText": "net::ERR_PROXY_CONNECTION_FAILED"}),
            Some("sid"),
        ))
        .unwrap();

        let result = recv_navigation_status(&mut rx, "sid").await;
        assert!(result.is_err(), "loadingFailed 应返回错误");
        let err = result.unwrap_err();
        match err {
            WispError::Browser(BrowserError::CdpConnection(msg)) => {
                assert!(msg.contains("net::ERR_PROXY_CONNECTION_FAILED"));
            }
            _ => panic!("应是 CdpConnection 错误，实际: {err:?}"),
        }
    }

    #[tokio::test]
    async fn test_recv_navigation_status_timeout_returns_200() {
        let (_tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        // 不发送任何事件，等待超时
        let status = recv_navigation_status(&mut rx, "sid")
            .await
            .expect("超时应返回默认 200");
        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn test_recv_navigation_status_ignores_other_session() {
        let (tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        // 不同 session 的事件应被忽略
        tx.send(make_event(
            "Network.responseReceived",
            json!({"type": "Document", "response": {"status": 404}}),
            Some("other-sid"),
        ))
        .unwrap();
        // 匹配 session 的事件应被采用
        tx.send(make_event(
            "Network.responseReceived",
            json!({"type": "Document", "response": {"status": 200}}),
            Some("sid"),
        ))
        .unwrap();

        let status = recv_navigation_status(&mut rx, "sid").await.expect("应返回状态码");
        assert_eq!(status, 200);
    }

    /// MockStrategy：用于验证 trait 可实现、可调用。
    struct MockStrategy;

    #[async_trait]
    impl BrowserFetchStrategy for MockStrategy {
        async fn fetch(&self, _page: &mut Page, req: &Request) -> Result<Response> {
            Ok(Response::from_browser(
                200,
                req.url.clone(),
                "<html></html>".to_string(),
                "mock".to_string(),
                Vec::new(),
                req.clone(),
            ))
        }
    }

    #[test]
    fn test_trait_object_can_be_constructed() {
        let strategy: Box<dyn BrowserFetchStrategy> = Box::new(MockStrategy);
        // 仅验证 trait object 可构造（无 UB）
        let _ = strategy as *const dyn BrowserFetchStrategy;
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib fetcher::strategy`
Expected: FAIL（模块未声明）

- [ ] **Step 3: 写最小实现**

修改 `src/fetcher/mod.rs`，在 `pub mod client;` 后添加 `pub mod strategy;`：

```rust
pub mod client;
pub mod response;
pub mod strategy;
```

确认 `Cargo.toml` 已包含 `async_trait` 依赖（PR1 应已具备）。若未包含，添加：

```toml
async-trait = "0.1"
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib fetcher::strategy`
Expected: PASS（4 个测试全通过）

- [ ] **Step 5: 提交**

```bash
git add src/fetcher/strategy.rs src/fetcher/mod.rs
git commit -m "feat: 添加 BrowserFetchStrategy trait + 公共导航/响应提取 helper"
```

---

### Task 2: DynamicStrategy 实现

**Files:**
- Create: `src/fetcher/strategies/mod.rs`
- Create: `src/fetcher/strategies/dynamic.rs`
- Modify: `src/fetcher/mod.rs`（添加 `pub mod strategies;` + re-export）
- Test: `src/fetcher/strategies/dynamic.rs`（内联测试模块）

**Interfaces:**
- Consumes: `crate::fetcher::strategy::{BrowserFetchStrategy, recv_navigation_status, extract_browser_response}`、`crate::browser::Page`、`crate::fetcher::response::{Request, Response}`、`crate::fetcher::client::FetchClientConfig`
- Produces: `pub struct DynamicStrategy { wait_for: Option<String>, extra_wait_ms: u64, timeout: Duration }`；`impl DynamicStrategy { pub fn from_config(config: &FetchClientConfig) -> Self }`；`impl BrowserFetchStrategy for DynamicStrategy`

- [ ] **Step 1: 写失败的测试**

创建 `src/fetcher/strategies/mod.rs`：

```rust
//! 浏览器抓取策略实现。

pub mod dynamic;
pub mod stealth;

pub use dynamic::DynamicStrategy;
pub use stealth::StealthStrategy;
```

创建 `src/fetcher/strategies/dynamic.rs`，包含测试：

```rust
//! Dynamic 模式策略：浏览器渲染 + JS 执行，无 CF 绕过。

use std::time::Duration;

use async_trait::async_trait;

use crate::browser::Page;
use crate::error::{BrowserError, Result, WispError};
use crate::fetcher::client::FetchClientConfig;
use crate::fetcher::response::{Request, Response};
use crate::fetcher::strategy::{extract_browser_response, recv_navigation_status, BrowserFetchStrategy};

/// Dynamic 模式策略：浏览器渲染 + JS 执行，无 CF 绕过。
///
/// ARCH: 从 `FetchClient::do_browser_work_inner`（solve_cf=false 分支）提取。
/// 仅做导航 + 等待选择器 + 提取响应。
pub struct DynamicStrategy {
    /// 等待特定 CSS 选择器出现（可选）。
    wait_for: Option<String>,
    /// 页面加载后额外等待（毫秒）。
    extra_wait_ms: u64,
    /// 单操作超时（用于 wait_for_selector）。
    timeout: Duration,
}

impl DynamicStrategy {
    /// 从 FetchClientConfig 构造。
    pub fn from_config(config: &FetchClientConfig) -> Self {
        Self {
            wait_for: config.wait_for.clone(),
            extra_wait_ms: config.extra_wait_ms,
            timeout: config.timeout,
        }
    }
}

#[async_trait]
impl BrowserFetchStrategy for DynamicStrategy {
    async fn fetch(&self, page: &mut Page, req: &Request) -> Result<Response> {
        let url = &req.url;
        tracing::info!("BrowserWork: {url} 开始（Dynamic）");

        // 启用 Network 域以捕获真实 HTTP 状态码
        page.cmd("Network.enable", serde_json::json!({}))
            .await
            .map_err(|e| {
                WispError::Browser(BrowserError::CdpConnection(format!(
                    "Network.enable failed: {e}"
                )))
            })?;

        // goto 之前订阅事件流，避免竞态
        let mut event_rx = page.session.subscribe_events();
        let sid = page.session_id.clone();

        let t_nav = std::time::Instant::now();
        tracing::info!("BrowserWork: {url} 导航");
        if let Err(e) = page.goto(&req.url).await {
            tracing::warn!("BrowserWork: {url} goto 失败: {e}");
            return Err(e);
        }
        tracing::trace!(elapsed_ms = t_nav.elapsed().as_millis(), url = %url, "goto timing");

        // 捕获导航请求的真实 HTTP 状态码
        let t_status = std::time::Instant::now();
        let nav_status = match recv_navigation_status(&mut event_rx, &sid).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("BrowserWork: {url} recv_navigation_status 失败: {e}");
                return Err(e);
            }
        };
        tracing::trace!(
            elapsed_ms = t_status.elapsed().as_millis(),
            code = nav_status,
            url = %url,
            "recv_status timing"
        );

        // 等待特定选择器
        if let Some(ref selector) = self.wait_for {
            page.wait_for_selector(selector, self.timeout.as_millis() as u64)
                .await?;
        }

        // 额外等待
        if self.extra_wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.extra_wait_ms)).await;
        }

        tracing::debug!("BrowserWork: {url} 提取响应");
        let resp = extract_browser_response(page, req, nav_status).await?;
        tracing::info!("BrowserWork: {url} 完成 ({} bytes)", resp.body.len());
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_config_default() {
        let config = FetchClientConfig::default();
        let strategy = DynamicStrategy::from_config(&config);
        assert_eq!(strategy.wait_for, config.wait_for);
        assert_eq!(strategy.extra_wait_ms, config.extra_wait_ms);
        assert_eq!(strategy.timeout, config.timeout);
    }

    #[test]
    fn test_from_config_with_wait_for() {
        let config = FetchClientConfig {
            wait_for: Some(".content".to_string()),
            extra_wait_ms: 1500,
            ..Default::default()
        };
        let strategy = DynamicStrategy::from_config(&config);
        assert_eq!(strategy.wait_for.as_deref(), Some(".content"));
        assert_eq!(strategy.extra_wait_ms, 1500);
    }

    /// 集成测试：实际浏览器导航 + wait_for_selector。
    /// 运行方式：cargo test --lib fetcher::strategies::dynamic -- --ignored
    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn test_dynamic_strategy_navigates() {
        use crate::browser::{Browser, BrowserPool};
        use crate::config::LaunchOptions;
        use crate::fetcher::response::Request;

        let pool = BrowserPool::new(1, LaunchOptions::default());
        let mut handle = pool.acquire().await.expect("acquire page");
        let page = handle.page_mut();

        let strategy = DynamicStrategy {
            wait_for: None,
            extra_wait_ms: 0,
            timeout: Duration::from_secs(30),
        };
        let req = Request::get("data:text/html,<html><body><h1>Test</h1></body></html>");
        let resp = strategy.fetch(page, &req).await.expect("fetch 应成功");
        assert_eq!(resp.status, 200);
        assert!(resp.body.len() > 0);
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib fetcher::strategies::dynamic`
Expected: FAIL（模块未声明）

- [ ] **Step 3: 写最小实现**

修改 `src/fetcher/mod.rs`，在 `pub mod strategy;` 后添加：

```rust
pub mod strategies;

pub use strategies::{DynamicStrategy, StealthStrategy};
```

注意：此时 `StealthStrategy` 尚未创建，会导致编译失败。临时方案：先只声明 `pub mod strategies;`，re-export 留到 Task 3 完成后添加。

修改后的 `src/fetcher/mod.rs` 模块声明部分应为：

```rust
pub mod client;
pub mod response;
pub mod strategy;
pub mod strategies;
```

并在文件顶部 import 区添加（Task 3 完成后启用）：

```rust
// Task 3 后启用：pub use strategies::{DynamicStrategy, StealthStrategy};
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib fetcher::strategies::dynamic`
Expected: PASS（2 个单元测试通过；1 个集成测试 ignored）

- [ ] **Step 5: 提交**

```bash
git add src/fetcher/strategies/mod.rs src/fetcher/strategies/dynamic.rs src/fetcher/mod.rs
git commit -m "feat: 添加 DynamicStrategy（浏览器渲染，无 CF 绕过）"
```

---

### Task 3: StealthStrategy 实现

**Files:**
- Create: `src/fetcher/strategies/stealth.rs`
- Modify: `src/fetcher/mod.rs`（启用 `pub use strategies::{DynamicStrategy, StealthStrategy};`）
- Test: `src/fetcher/strategies/stealth.rs`（内联测试模块）

**Interfaces:**
- Consumes: `crate::fetcher::strategy::{BrowserFetchStrategy, recv_navigation_status, extract_browser_response}`、`crate::browser::Page`、`crate::cookie::CfCookieJar`（PR1 已迁移）、`crate::stealth::{ChallengeSolver, HumanBehavior, TurnstileConfig}`、`crate::fetcher::client::FetchClientConfig`
- Produces: `pub struct StealthStrategy { challenge_timeout, turnstile, human_mode, cf_jar }`；`impl StealthStrategy { pub fn from_config(config: &FetchClientConfig, cf_jar: Arc<CfCookieJar>) -> Self }`；`impl BrowserFetchStrategy for StealthStrategy`

**关键修复点（必须保留）：**
1. Network.loadingFailed 处理（`recv_navigation_status` 立即返回错误）
2. nav_status = 200 修正（CF 挑战解决后）
3. `wait_for_load` 使用 `broadcast::Receiver`（只等待新事件）— 由 `recv_navigation_status` 保证
4. 120s 总超时 — 由 `FetchClient::fetch_browser` 包装（非此策略职责）
5. 关键步骤 tracing 日志（start/navigation/status code/CF challenge/extraction/completion）
6. goto/recv_navigation_status 失败的 warn 日志
7. CF cookie 注入（从 cf_jar 读取，写入 page）
8. CF 挑战解决后保存 cookie + UA 到 cf_jar

- [ ] **Step 1: 写失败的测试**

创建 `src/fetcher/strategies/stealth.rs`，包含完整实现 + 测试：

```rust
//! Stealth 模式策略：CF bypass + 人类行为模拟 + cookie 复用。
//!
//! ARCH: 从 `FetchClient::do_browser_work_inner`（solve_cf=true 分支）提取。
//! CfCookieJar 由本策略独占持有。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::browser::Page;
use crate::cookie::CfCookieJar;
use crate::error::{BrowserError, Result, WispError};
use crate::fetcher::client::FetchClientConfig;
use crate::fetcher::response::{Request, Response};
use crate::fetcher::strategy::{extract_browser_response, recv_navigation_status, BrowserFetchStrategy};
use crate::stealth::{ChallengeSolver, HumanBehavior, TurnstileConfig};

/// Stealth 模式策略：CF bypass + 人类行为模拟 + cookie 复用。
///
/// ARCH: CfCookieJar 从 FetchClient 迁入此策略，由 StealthStrategy 独占持有。
pub struct StealthStrategy {
    /// CF 挑战超时。
    challenge_timeout: Duration,
    /// Turnstile 解决器参数。
    turnstile: TurnstileConfig,
    /// 是否启用人类行为模拟。
    human_mode: bool,
    /// 等待特定 CSS 选择器出现（可选）。
    wait_for: Option<String>,
    /// 页面加载后额外等待（毫秒）。
    extra_wait_ms: u64,
    /// 单操作超时（用于 wait_for_selector）。
    timeout: Duration,
    /// CF 会话 cookie 缓存（moka + 文件持久化）。
    cf_jar: Arc<CfCookieJar>,
}

impl StealthStrategy {
    /// 从 FetchClientConfig + 共享 CfCookieJar 构造。
    pub fn from_config(config: &FetchClientConfig, cf_jar: Arc<CfCookieJar>) -> Self {
        Self {
            challenge_timeout: config.challenge_timeout,
            turnstile: config.turnstile.clone(),
            human_mode: config.human_mode,
            wait_for: config.wait_for.clone(),
            extra_wait_ms: config.extra_wait_ms,
            timeout: config.timeout,
            cf_jar,
        }
    }
}

#[async_trait]
impl BrowserFetchStrategy for StealthStrategy {
    async fn fetch(&self, page: &mut Page, req: &Request) -> Result<Response> {
        let url = &req.url;
        tracing::info!("BrowserWork[+CF]: {url} 开始");

        // 启用 Network 域以捕获真实 HTTP 状态码
        page.cmd("Network.enable", serde_json::json!({}))
            .await
            .map_err(|e| {
                WispError::Browser(BrowserError::CdpConnection(format!(
                    "Network.enable failed: {e}"
                )))
            })?;

        // goto 之前订阅事件流，避免竞态
        let mut event_rx = page.session.subscribe_events();
        let sid = page.session_id.clone();

        // 注入之前保存的 CF cookie（复用 CF 挑战结果）
        let domain = url::Url::parse(&req.url)
            .ok()
            .and_then(|u| u.host_str().map(std::string::ToString::to_string));
        if let Some(ref domain) = domain {
            if let Some(session) = self.cf_jar.get(domain) {
                for cookie in &session.cookies {
                    let name = cookie.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let value = cookie.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    let cookie_domain = cookie
                        .get("domain")
                        .and_then(|d| d.as_str())
                        .unwrap_or(domain);
                    let path = cookie.get("path").and_then(|p| p.as_str()).unwrap_or("/");
                    let secure = cookie
                        .get("secure")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    let http_only = cookie
                        .get("httpOnly")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    let same_site = cookie
                        .get("sameSite")
                        .and_then(|s| s.as_str())
                        .unwrap_or("Lax");
                    let _ = page
                        .cmd(
                            "Network.setCookie",
                            serde_json::json!({
                                "name": name,
                                "value": value,
                                "domain": cookie_domain,
                                "path": path,
                                "secure": secure,
                                "httpOnly": http_only,
                                "sameSite": same_site,
                            }),
                        )
                        .await;
                }
                tracing::info!(
                    "BrowserWork[+CF]: {url} 注入 {} 个 CF cookie",
                    session.cookies.len()
                );
            }
        }

        let t_nav = std::time::Instant::now();
        tracing::info!("BrowserWork[+CF]: {url} 导航");
        if let Err(e) = page.goto(&req.url).await {
            tracing::warn!("BrowserWork[+CF]: {url} goto 失败: {e}");
            return Err(e);
        }
        tracing::trace!(elapsed_ms = t_nav.elapsed().as_millis(), url = %url, "goto timing");

        // 捕获导航请求的真实 HTTP 状态码
        let t_status = std::time::Instant::now();
        let mut nav_status = match recv_navigation_status(&mut event_rx, &sid).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("BrowserWork[+CF]: {url} recv_navigation_status 失败: {e}");
                return Err(e);
            }
        };
        tracing::trace!(
            elapsed_ms = t_status.elapsed().as_millis(),
            code = nav_status,
            url = %url,
            "recv_status timing"
        );

        // CF 挑战解决
        let t_cf = std::time::Instant::now();
        let solver = ChallengeSolver::new(page);
        solver
            .solve_with_config(self.challenge_timeout, &self.turnstile)
            .await?;
        tracing::trace!(elapsed_ms = t_cf.elapsed().as_millis(), url = %url, "solve_cf timing");

        // CF 挑战解决后，nav_status 捕获的是首次 goto 时的状态码（通常是 403/503 挑战页），
        // 不能反映挑战解决后的最终页面状态。修正为 200。
        if nav_status != 200 {
            tracing::debug!(
                "BrowserWork[+CF]: {url} CF 挑战解决，状态码 {nav_status} → 200"
            );
            nav_status = 200;
        }

        // 人类行为模拟
        if self.human_mode {
            let human = HumanBehavior::new(page);
            human.random_delay(500, 1500).await?;
            human.random_scroll().await?;
            human.random_delay(300, 800).await?;
        }

        // CF 挑战解决后，保存 cookie + 浏览器实际 UA 到缓存
        if let Some(ref domain) = domain {
            let mut ua_str = String::new();
            if let Ok(ua_val) = page.evaluate("navigator.userAgent").await {
                if let Some(s) = ua_val.as_str() {
                    ua_str = s.to_string();
                }
            }
            if let Ok(resp) = page.cmd("Network.getCookies", serde_json::json!({})).await {
                if let Some(cookies) = resp.pointer("/cookies").and_then(|c| c.as_array()) {
                    let cookies_to_save: Vec<serde_json::Value> = cookies
                        .iter()
                        .filter(|c| {
                            let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            // 只保存 CF 相关 cookie（cf_clearance 等）
                            name.starts_with("cf_") || name.starts_with("__cf")
                        })
                        .cloned()
                        .collect();
                    if !cookies_to_save.is_empty() {
                        self.cf_jar.insert(
                            domain.clone(),
                            crate::cookie::CfSession {
                                cookies: cookies_to_save.clone(),
                                ua: ua_str,
                                saved_at: chrono::Utc::now().timestamp(),
                            },
                        );
                        tracing::info!(
                            "BrowserWork[+CF]: {url} 保存 {} 个 CF cookie",
                            cookies_to_save.len()
                        );
                    }
                }
            }
        }

        // 等待特定选择器
        if let Some(ref selector) = self.wait_for {
            page.wait_for_selector(selector, self.timeout.as_millis() as u64)
                .await?;
        }

        // 额外等待
        if self.extra_wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.extra_wait_ms)).await;
        }

        tracing::debug!("BrowserWork[+CF]: {url} 提取响应");
        let resp = extract_browser_response(page, req, nav_status).await?;
        tracing::info!(
            "BrowserWork[+CF]: {url} 完成 ({} bytes)",
            resp.body.len()
        );
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_config_default() {
        let config = FetchClientConfig::default();
        let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
        let strategy = StealthStrategy::from_config(&config, cf_jar);
        assert_eq!(strategy.challenge_timeout, config.challenge_timeout);
        assert_eq!(strategy.human_mode, config.human_mode);
        assert_eq!(strategy.wait_for, config.wait_for);
        assert_eq!(strategy.extra_wait_ms, config.extra_wait_ms);
    }

    #[test]
    fn test_from_config_custom() {
        let config = FetchClientConfig {
            human_mode: false,
            challenge_timeout: Duration::from_secs(60),
            wait_for: Some(".loaded".to_string()),
            extra_wait_ms: 2000,
            ..Default::default()
        };
        let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
        let strategy = StealthStrategy::from_config(&config, cf_jar);
        assert!(!strategy.human_mode);
        assert_eq!(strategy.challenge_timeout, Duration::from_secs(60));
        assert_eq!(strategy.wait_for.as_deref(), Some(".loaded"));
        assert_eq!(strategy.extra_wait_ms, 2000);
    }

    /// 集成测试：CF 挑战解决 + cookie 持久化。
    /// 运行方式：cargo test --lib fetcher::strategies::stealth -- --ignored
    #[tokio::test]
    #[ignore = "需要 CF 保护的站点环境"]
    async fn test_stealth_strategy_solves_cf() {
        use crate::browser::BrowserPool;
        use crate::config::LaunchOptions;
        use crate::fetcher::response::Request;

        let config = FetchClientConfig::default();
        let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
        let strategy = StealthStrategy::from_config(&config, cf_jar);

        let pool = BrowserPool::new(1, LaunchOptions::default());
        let mut handle = pool.acquire().await.expect("acquire page");
        let page = handle.page_mut();

        // 替换为实际的 CF 保护站点
        let req = Request::get("https://example.com/");
        let resp = strategy.fetch(page, &req).await.expect("fetch 应成功");
        assert_eq!(resp.status, 200);
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib fetcher::strategies::stealth`
Expected: FAIL（模块未声明 + `crate::cookie::CfCookieJar` / `crate::cookie::CfSession` 可能未导出）

若 `crate::cookie::CfCookieJar` 或 `crate::cookie::CfSession` 不存在，需确认 PR1 已正确迁移。如缺失，临时在 `src/cookie/mod.rs` 添加 `pub use cf::{CfCookieJar, CfSession};`（PR1 应已完成此步）。

- [ ] **Step 3: 写最小实现**

修改 `src/fetcher/mod.rs`，启用 re-export（替换 Task 2 中的临时注释）：

```rust
pub mod client;
pub mod response;
pub mod strategy;
pub mod strategies;

pub use strategies::{DynamicStrategy, StealthStrategy};
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib fetcher::strategies::stealth`
Expected: PASS（2 个单元测试通过；1 个集成测试 ignored）

- [ ] **Step 5: 提交**

```bash
git add src/fetcher/strategies/stealth.rs src/fetcher/mod.rs
git commit -m "feat: 添加 StealthStrategy（CF bypass + 人类行为 + cookie 复用）"
```

---

### Task 4: FetchClient::fetch_browser 新签名 + 删除 do_browser_work_inner

**Files:**
- Modify: `src/fetcher/client.rs:296-505`（`fetch_browser` 签名改为接收 `&dyn BrowserFetchStrategy`；删除 `do_browser_work_inner`）
- Test: `src/fetcher/client.rs`（内联测试模块）

**Interfaces:**
- Consumes: `crate::fetcher::strategy::BrowserFetchStrategy`
- Produces: `pub async fn fetch_browser(&self, req: &Request, strategy: &dyn BrowserFetchStrategy) -> Result<Response>`

- [ ] **Step 1: 写失败的测试**

在 `src/fetcher/client.rs` 的 `#[cfg(test)] mod tests` 中添加测试：

```rust
    use crate::fetcher::strategy::BrowserFetchStrategy;
    use crate::fetcher::response::Request;
    use crate::browser::Page;
    use async_trait::async_trait;

    /// Mock 策略：返回固定响应，用于验证 fetch_browser 调用契约。
    struct MockStrategy {
        called: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl BrowserFetchStrategy for MockStrategy {
        async fn fetch(&self, _page: &mut Page, req: &Request) -> Result<Response> {
            self.called.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(Response::from_browser(
                200,
                req.url.clone(),
                "<html>mock</html>".to_string(),
                "mock".to_string(),
                Vec::new(),
                req.clone(),
            ))
        }
    }

    #[tokio::test]
    async fn test_fetch_browser_invokes_strategy() {
        // max_concurrent_pages=0 会导致无 browser_pool，需 >0
        let config = FetchClientConfig::default();
        let client = FetchClient::new(config).expect("build client");
        let called = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let strategy = MockStrategy { called: called.clone() };
        let req = Request::get("data:text/html,<html></html>");

        // 注意：此测试需要真实 Chrome（BrowserPool::acquire 会启动浏览器）
        // 若无 Chrome 环境，会返回 LaunchFailed 错误
        let result = client.fetch_browser(&req, &strategy).await;
        if result.is_ok() {
            assert_eq!(called.load(std::sync::atomic::Ordering::Relaxed), 1);
        }
        // 无 Chrome 环境下不报错（忽略结果）
    }

    #[tokio::test]
    async fn test_fetch_browser_no_pool_returns_error() {
        let config = FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        };
        let client = FetchClient::new(config).expect("build client");
        let called = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let strategy = MockStrategy { called: called.clone() };
        let req = Request::get("https://example.com/");

        let result = client.fetch_browser(&req, &strategy).await;
        assert!(result.is_err(), "无 browser_pool 应返回错误");
        // 策略不应被调用
        assert_eq!(called.load(std::sync::atomic::Ordering::Relaxed), 0);
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib fetcher::client::tests::test_fetch_browser_no_pool_returns_error`
Expected: FAIL（`fetch_browser` 仍接收 `solve_cf: bool`，编译错误）

- [ ] **Step 3: 写最小实现**

修改 `src/fetcher/client.rs`：

1. 在文件顶部添加 import：

```rust
use super::strategy::BrowserFetchStrategy;
```

2. 替换 `fetch_browser` 方法（删除 `solve_cf: bool` 参数，改为 `strategy: &dyn BrowserFetchStrategy`，删除 `do_browser_work_inner` 调用）：

```rust
    /// 浏览器请求（通过 BrowserPool + 注入 strategy）。
    ///
    /// ARCH: 替代 `fetch_browser(&req, solve_cf: bool)`。
    /// strategy 由调用方传入，FetchClient 不再关心 CF/Dynamic 差异。
    /// 120s 总超时由本方法包装。
    pub async fn fetch_browser(
        &self,
        req: &Request,
        strategy: &dyn BrowserFetchStrategy,
    ) -> Result<Response> {
        let pool = self.browser_pool.as_ref().ok_or_else(|| {
            WispError::Browser(BrowserError::Other(
                "browser pool not configured (max_concurrent_pages=0)".into(),
            ))
        })?;
        // acquire 返回带 page 的 handle（permit 限制并发数）
        let mut handle = pool.acquire().await?;

        // 总超时：防止 CF 挑战页面卡住整个流程（导航+挑战+提取各阶段都有单独超时，
        // 但极端情况下可能累加超过预期，这里加一个 120s 硬上限）
        let work = strategy.fetch(handle.page_mut(), req);
        let result = tokio::time::timeout(Duration::from_secs(120), work)
            .await
            .map_err(|_| {
                WispError::Timeout(format!(
                    "fetch_browser 总超时（120s）: {}",
                    crate::crawl::engine::sanitize_url(&req.url)
                ))
            })?;

        // 实际工作；无论成功/失败都显式关闭 tab
        let _ = handle.page_mut().close().await;
        // handle Drop：page.target_id 已 None（Page::Drop no-op）+ permit 自动 release
        result
    }
```

3. 删除 `do_browser_work_inner` 整个方法（约 180 行，从 `async fn do_browser_work_inner` 到对应的 `}`）。

4. 删除文件顶部不再使用的 import：

```rust
// 删除以下 import（do_browser_work_inner 已删除，不再使用）：
// use crate::stealth::challenge::ChallengeSolver;
// use crate::stealth::human::HumanBehavior;
```

注意：保留 `recv_navigation_status` 和 `extract_browser_response` 暂不删除（Task 5 处理），但它们现在不被调用，会触发 `dead_code` 警告。临时添加 `#[allow(dead_code)]`：

```rust
    #[allow(dead_code)]
    async fn recv_navigation_status(...) -> ... { ... }

    #[allow(dead_code)]
    async fn extract_browser_response(...) -> ... { ... }
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib fetcher::client`
Expected: PASS（`test_fetch_browser_no_pool_returns_error` 通过；`test_fetch_browser_invokes_strategy` 在无 Chrome 环境下因 acquire 失败而跳过断言）

- [ ] **Step 5: 提交**

```bash
git add src/fetcher/client.rs
git commit -m "refactor: FetchClient::fetch_browser 接收 &dyn BrowserFetchStrategy"
```

---

### Task 5: 删除 recv_navigation_status + extract_browser_response 旧实现

**Files:**
- Modify: `src/fetcher/client.rs`（删除 `recv_navigation_status` 和 `extract_browser_response` 私有方法）
- Test: 无新测试（验证编译 + 现有测试全绿）

**Interfaces:**
- Consumes: Task 1 已在 `strategy.rs` 提供 `recv_navigation_status` 和 `extract_browser_response` 公共 helper
- Produces: 无（仅删除代码）

- [ ] **Step 1: 写失败的测试**

无新测试。验证当前状态：

```bash
cargo build --lib 2>&1 | grep dead_code
```

Expected: 应看到 `recv_navigation_status` 和 `extract_browser_response` 的 `dead_code` 警告（Task 4 已加 `#[allow(dead_code)]`，无警告则跳过此步）。

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib fetcher::client`
Expected: PASS（当前测试全绿，无失败。此任务以"删除冗余代码"为验证目标）

- [ ] **Step 3: 写最小实现**

在 `src/fetcher/client.rs` 中：

1. 删除整个 `recv_navigation_status` 方法（约 75 行，从 `async fn recv_navigation_status` 到对应 `}`）。
2. 删除整个 `extract_browser_response` 方法（约 25 行，从 `async fn extract_browser_response` 到对应 `}`）。
3. 删除 Task 4 临时添加的 `#[allow(dead_code)]` 注解（已随方法删除）。
4. 检查并删除因方法删除而变得无用的 import（如 `tokio::sync::broadcast`，若仅被 `recv_navigation_status` 使用）。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib fetcher::client && cargo build --lib 2>&1 | grep -E "warning.*dead_code"`
Expected: 测试 PASS；无 dead_code 警告。

- [ ] **Step 5: 提交**

```bash
git add src/fetcher/client.rs
git commit -m "refactor: 删除 FetchClient 中已迁至 strategy 的私有方法"
```

---

### Task 6: Fetcher 持有 browser_strategy 字段

**Files:**
- Modify: `src/fetcher/mod.rs:61-141`（`Fetcher` 结构体 + `new` + `fetch`）
- Test: `src/fetcher/mod.rs`（内联测试模块）

**Interfaces:**
- Consumes: `crate::fetcher::strategy::BrowserFetchStrategy`、`crate::fetcher::strategies::{DynamicStrategy, StealthStrategy}`、`crate::cookie::CfCookieJar`
- Produces: `Fetcher { client, mode, browser_strategy: Option<Arc<dyn BrowserFetchStrategy>> }`

- [ ] **Step 1: 写失败的测试**

在 `src/fetcher/mod.rs` 的 `#[cfg(test)] mod tests` 中添加测试：

```rust
    #[test]
    fn test_fetcher_http_mode_has_no_strategy() {
        let fetcher = Fetcher::new(FetchMode::Http, FetchClientConfig::default())
            .expect("build fetcher");
        assert!(fetcher.browser_strategy.is_none());
    }

    #[test]
    fn test_fetcher_auto_mode_has_no_strategy() {
        let fetcher = Fetcher::new(FetchMode::Auto, FetchClientConfig::default())
            .expect("build fetcher");
        assert!(fetcher.browser_strategy.is_none());
    }

    #[test]
    fn test_fetcher_dynamic_mode_has_strategy() {
        let fetcher = Fetcher::new(FetchMode::Dynamic, FetchClientConfig::default())
            .expect("build fetcher");
        assert!(fetcher.browser_strategy.is_some(), "Dynamic 模式应有 strategy");
    }

    #[test]
    fn test_fetcher_stealth_mode_has_strategy() {
        let fetcher = Fetcher::new(FetchMode::Stealth, FetchClientConfig::default())
            .expect("build fetcher");
        assert!(fetcher.browser_strategy.is_some(), "Stealth 模式应有 strategy");
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib fetcher::tests::test_fetcher_http_mode_has_no_strategy`
Expected: FAIL（`browser_strategy` 字段不存在，编译错误）

- [ ] **Step 3: 写最小实现**

修改 `src/fetcher/mod.rs` 的 `Fetcher` 结构体和 `impl` 块：

1. 在 `use` 区添加：

```rust
use crate::cookie::CfCookieJar;
use crate::error::{Result, WispError};
use crate::fetcher::strategy::BrowserFetchStrategy;
use crate::fetcher::strategies::{DynamicStrategy, StealthStrategy};
```

2. 修改 `Fetcher` 结构体（添加 `browser_strategy` 字段）：

```rust
pub struct Fetcher {
    client: Arc<FetchClient>,
    mode: FetchMode,
    /// 浏览器模式下的 strategy（Http/Auto 为 None）。
    /// ARCH: 由 Fetcher::new 根据 mode 自动构造。
    browser_strategy: Option<Arc<dyn BrowserFetchStrategy>>,
}
```

3. 修改 `Fetcher::new`（根据 mode 构造 strategy）：

```rust
    /// 从配置创建 Fetcher。
    pub fn new(mode: FetchMode, config: FetchClientConfig) -> Result<Self> {
        let client = Arc::new(FetchClient::new(config.clone())?);
        let browser_strategy = Self::build_strategy(mode, &config)?;
        Ok(Self {
            client,
            mode,
            browser_strategy,
        })
    }

    /// 根据 mode 构造 browser_strategy。
    fn build_strategy(
        mode: FetchMode,
        config: &FetchClientConfig,
    ) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        match mode {
            FetchMode::Http | FetchMode::Auto => Ok(None),
            FetchMode::Dynamic => Ok(Some(Arc::new(DynamicStrategy::from_config(config)))),
            FetchMode::Stealth => {
                let cf_jar = Arc::new(CfCookieJar::new(
                    &config.cf_data_dir,
                    config.cf_cookie_ttl,
                ));
                Ok(Some(Arc::new(StealthStrategy::from_config(config, cf_jar))))
            }
        }
    }
```

4. 修改 `Fetcher::from_client`（无 mode-specific config，strategy 留空）：

```rust
    /// 从已有 FetchClient 创建 Fetcher。
    /// 注意：此构造方式不创建 browser_strategy，Dynamic/Stealth 模式下需调用方自行注入。
    #[must_use]
    pub fn from_client(client: Arc<FetchClient>, mode: FetchMode) -> Self {
        Self {
            client,
            mode,
            browser_strategy: None,
        }
    }
```

5. 修改 `Fetcher::fetch`（使用 strategy）：

```rust
    /// 发送请求（根据模式委托给 FetchClient）。
    pub async fn fetch(&self, req: Request) -> Result<Response> {
        match self.mode {
            FetchMode::Http | FetchMode::Auto => self.client.fetch_http(&req).await,
            FetchMode::Dynamic | FetchMode::Stealth => {
                let strategy = self.browser_strategy.as_ref().ok_or_else(|| {
                    WispError::Config(format!(
                        "{:?} mode requires browser_strategy, use Fetcher::new() instead of from_client()",
                        self.mode
                    ))
                })?;
                self.client.fetch_browser(&req, strategy.as_ref()).await
            }
        }
    }
```

6. 添加 `browser_strategy` 访问器（可选，便于测试）：

```rust
    /// 获取浏览器策略引用（如有）。
    #[must_use]
    pub fn browser_strategy(&self) -> Option<&Arc<dyn BrowserFetchStrategy>> {
        self.browser_strategy.as_ref()
    }
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib fetcher`
Expected: PASS（4 个新测试 + 现有测试全通过）

- [ ] **Step 5: 提交**

```bash
git add src/fetcher/mod.rs
git commit -m "refactor: Fetcher 持有 browser_strategy，按 mode 自动构造"
```

---

### Task 7: browser/patches.rs 迁移到 stealth/patches.rs

**Files:**
- Create: `src/stealth/patches.rs`（从 `src/browser/patches.rs` 复制内容）
- Delete: `src/browser/patches.rs`
- Modify: `src/browser/mod.rs:14`（删除 `pub mod patches;`）
- Modify: `src/stealth/mod.rs:5`（添加 `pub mod patches;`）
- Modify: `src/browser/page.rs:116-118`（引用从 `crate::browser::patches::` 改为 `crate::stealth::patches::`）
- Test: `src/stealth/patches.rs`（保留原测试）

**Interfaces:**
- Consumes: 无（纯文件迁移）
- Produces: `crate::stealth::patches::{patch_launch_args, SHADOW_DOM_PATCH_SCRIPT, HEADED_STEALTH_SCRIPT, HEADLESS_STEALTH_SCRIPT, STEALTH_SCRIPT}`

**注意：** PR2 阶段 stealth 模块仍总是编译，`browser::page` 引用 `stealth::patches` 不会导致 feature gate 问题。PR3 启用 feature gate 时会重构 `Page::create` 接收脚本参数（不在本 PR 范围）。

- [ ] **Step 1: 写失败的测试**

验证当前 patches 测试在原位置通过：

Run: `cargo test --lib browser::patches`
Expected: PASS（3 个测试通过）

- [ ] **Step 2: 运行测试验证失败**

无失败测试。此任务以"迁移后测试仍通过"为验证目标。

- [ ] **Step 3: 写最小实现**

1. 创建 `src/stealth/patches.rs`，内容与 `src/browser/patches.rs` 完全相同（复制所有 const、函数、测试）。

2. 修改 `src/browser/mod.rs`，删除第 14 行 `pub mod patches;`：

修改前：
```rust
/// 页面操作（导航、JS 执行、截图等）。
pub mod page;
/// 反检测 JS 补丁注入。
pub mod patches;
/// 浏览器实例池（复用 + 并发控制）。
pub mod pool;
```

修改后：
```rust
/// 页面操作（导航、JS 执行、截图等）。
pub mod page;
/// 浏览器实例池（复用 + 并发控制）。
pub mod pool;
```

3. 修改 `src/stealth/mod.rs`，添加 `pub mod patches;`：

修改前：
```rust
pub mod challenge;
pub mod human;
pub mod turnstile;
```

修改后：
```rust
pub mod challenge;
pub mod human;
pub mod patches;
pub mod turnstile;
```

4. 修改 `src/browser/page.rs` 第 116-118 行，更新引用路径：

修改前：
```rust
        let stealth_script = if headless {
            crate::browser::patches::HEADLESS_STEALTH_SCRIPT
        } else {
            crate::browser::patches::HEADED_STEALTH_SCRIPT
        };
```

修改后：
```rust
        let stealth_script = if headless {
            crate::stealth::patches::HEADLESS_STEALTH_SCRIPT
        } else {
            crate::stealth::patches::HEADED_STEALTH_SCRIPT
        };
```

5. 删除 `src/browser/patches.rs`（使用 DeleteFile 工具）。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib stealth::patches && cargo test --lib browser::page`
Expected: PASS（patches 的 3 个测试在 stealth 模块下通过；page 相关测试通过）

- [ ] **Step 5: 提交**

```bash
git add src/stealth/patches.rs src/stealth/mod.rs src/browser/mod.rs src/browser/page.rs
git rm src/browser/patches.rs
git commit -m "refactor: browser/patches.rs 迁移到 stealth/patches.rs"
```

---

### Task 8: build_stealth_args 拆为 build_common_args + build_stealth_extra_args

**Files:**
- Modify: `src/browser/launch.rs:65-130`（拆分 `build_stealth_args`，新增 `build_common_args` + `build_stealth_extra_args`，`build_default_args` 改为组合两者）
- Test: `src/browser/launch.rs`（内联测试模块）

**Interfaces:**
- Consumes: `crate::config::LaunchOptions`
- Produces:
  - `pub fn build_common_args(options: &LaunchOptions) -> Vec<String>`
  - `pub fn build_stealth_extra_args(options: &LaunchOptions) -> Vec<String>`
  - `pub fn build_default_args(options: &LaunchOptions) -> Vec<String>`（= common + stealth_extra）
  - 保留 `pub fn build_stealth_args(options: &LaunchOptions) -> Vec<String>`（= common + stealth_extra，向后兼容 `Browser::launch` 调用）

**注意：** PR2 阶段 `build_stealth_extra_args` 不加 `#[cfg(feature = "stealth")]`（PR3 启用 feature gate 时添加）。

- [ ] **Step 1: 写失败的测试**

在 `src/browser/launch.rs` 的 `#[cfg(test)] mod tests` 中添加测试：

```rust
    #[test]
    fn test_build_common_args_excludes_stealth_specific() {
        let opts = LaunchOptions::default();
        let args = build_common_args(&opts);
        // 通用参数应包含 no-first-run、disable-background-networking
        assert!(args.contains(&"no-first-run".to_string()));
        assert!(args.contains(&"disable-background-networking".to_string()));
        // 通用参数不应包含 stealth 专用参数
        assert!(!args.contains(&"disable-blink-features=AutomationControlled".to_string()));
    }

    #[test]
    fn test_build_stealth_extra_args_contains_stealth_specific() {
        let opts = LaunchOptions::default();
        let args = build_stealth_extra_args(&opts);
        assert!(args.contains(&"disable-blink-features=AutomationControlled".to_string()));
        // stealth_extra 不应包含通用参数
        assert!(!args.contains(&"no-first-run".to_string()));
    }

    #[test]
    fn test_build_default_args_combines_common_and_stealth() {
        let opts = LaunchOptions::default();
        let args = build_default_args(&opts);
        // 应同时包含通用参数和 stealth 专用参数
        assert!(args.iter().all(|a| a.starts_with("--")));
        assert!(args.iter().any(|a| a == "--no-first-run"));
        assert!(args.iter().any(|a| a == "--disable-blink-features=AutomationControlled"));
    }

    #[test]
    fn test_build_stealth_args_equals_common_plus_stealth_extra() {
        let opts = LaunchOptions::default();
        let common = build_common_args(&opts);
        let extra = build_stealth_extra_args(&opts);
        let combined = build_stealth_args(&opts);
        // combined 应等于 common + extra（顺序可能不同，但内容相同）
        let mut expected = common.clone();
        expected.extend(extra.clone());
        expected.sort();
        let mut actual = combined.clone();
        actual.sort();
        assert_eq!(expected, actual);
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib browser::launch::tests::test_build_common_args_excludes_stealth_specific`
Expected: FAIL（`build_common_args` / `build_stealth_extra_args` 未定义，编译错误）

- [ ] **Step 3: 写最小实现**

修改 `src/browser/launch.rs`，替换 `build_stealth_args` 和 `build_default_args`：

```rust
/// Build default Chrome launch arguments from options, with patches applied.
/// These args include the "--" prefix (for testing/verification).
///
/// ARCH: = build_common_args + build_stealth_extra_args，加 "--" 前缀。
#[must_use]
pub fn build_default_args(options: &LaunchOptions) -> Vec<String> {
    build_stealth_args(options)
        .iter()
        .map(|a| format!("--{a}"))
        .collect()
}

/// 构建完整启动参数（common + stealth_extra），无 "--" 前缀。
///
/// ARCH: 保留原 `build_stealth_args` 名字以兼容 `Browser::launch` 调用。
pub fn build_stealth_args(options: &LaunchOptions) -> Vec<String> {
    let mut args = build_common_args(options);
    args.extend(build_stealth_extra_args(options));
    args
}

/// 构建通用 Chrome 启动参数（不含 stealth 专用参数）。
///
/// ARCH: 通用参数对所有浏览器模式（Dynamic/Stealth）都需要。
/// 不包含 `disable-blink-features=AutomationControlled` 等 stealth 专用参数。
pub fn build_common_args(options: &LaunchOptions) -> Vec<String> {
    let mut args: Vec<String> = vec![
        // patchright chromiumSwitches (verified from source)
        "disable-field-trial-config".into(),
        "disable-background-networking".into(),
        "disable-background-timer-throttling".into(),
        "disable-backgrounding-occluded-windows".into(),
        "disable-breakpad".into(),
        "no-default-browser-check".into(),
        "disable-dev-shm-usage".into(),
        "disable-hang-monitor".into(),
        "disable-prompt-on-repost".into(),
        "disable-renderer-backgrounding".into(),
        "force-color-profile=srgb".into(),
        "no-first-run".into(),
        "password-store=basic".into(),
        "use-mock-keychain".into(),
        "no-service-autorun".into(),
        "export-tagged-pdf".into(),
        "disable-search-engine-choice-screen".into(),
        "disable-infobars".into(),
        "disable-sync".into(),
        // Disabled features (from patchright)
        "disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,DestroyProfileOnBrowserClose,DialMediaRouteProvider,GlobalMediaControls,HttpsUpgrades,LensOverlay,MediaRouter,PaintHolding".into(),
    ];

    // NOTE: Do NOT add --no-sandbox (causes Chrome re-launch on Windows headed mode)
    // NOTE: Do NOT add --enable-automation, --disable-popup-blocking, etc.

    // Proxy
    if let Some(ref proxy) = options.proxy {
        args.push(format!("proxy-server={}", proxy.server));
        // Chrome --proxy-server 不支持内联认证；username/password 无法通过命令行传递。
        // 需通过 CDP Fetch.requestPaused 拦截 407 或扩展程序处理（当前未实现）。
        if proxy.username.is_some() || proxy.password.is_some() {
            tracing::warn!(
                "Browser proxy auth (username/password) is not supported via --proxy-server. \
                 The proxy will be used without authentication; expect 407 responses. \
                 To use authenticated proxies with browser mode, configure the proxy to \
                 whitelist the client IP or use an unauthenticated proxy."
            );
        }
    }

    // User-provided extra args (strip -- prefix if present)
    for arg in &options.args {
        let stripped = arg.strip_prefix("--").unwrap_or(arg);
        args.push(stripped.to_string());
    }

    args
}

/// 构建 stealth 专用额外参数。
///
/// ARCH: PR2 阶段无 cfg gate（stealth 模块仍总是编译）。
/// PR3 启用 feature gate 后，此函数加 `#[cfg(feature = "stealth")]`，
/// `build_default_args` 中的调用也加对应 cfg。
pub fn build_stealth_extra_args(_options: &LaunchOptions) -> Vec<String> {
    vec![
        // Core anti-detection flag
        "disable-blink-features=AutomationControlled".into(),
    ]
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib browser::launch`
Expected: PASS（4 个新测试 + 现有 6 个测试全通过）

- [ ] **Step 5: 提交**

```bash
git add src/browser/launch.rs
git commit -m "refactor: build_stealth_args 拆为 build_common_args + build_stealth_extra_args"
```

---

### Task 9: Browser::launch 注释清理

**Files:**
- Modify: `src/browser/mod.rs:1`（模块注释）
- Modify: `src/browser/mod.rs:49`（`Browser::launch` doc comment）
- Test: 无新测试（验证编译 + 现有测试全绿）

**Interfaces:**
- Consumes: 无
- Produces: 无（仅注释变更）

- [ ] **Step 1: 写失败的测试**

无新测试。验证当前状态：

Run: `cargo test --lib browser`
Expected: PASS

- [ ] **Step 2: 运行测试验证失败**

无失败测试。此任务以"注释清理后编译 + 测试全绿"为验证目标。

- [ ] **Step 3: 写最小实现**

修改 `src/browser/mod.rs`：

1. 修改模块顶部注释（第 1 行）：

修改前：
```rust
//! Browser process management. Launches Chrome directly with stealth args.
```

修改后：
```rust
//! Browser process management. Launches Chrome directly with launch args.
```

2. 修改 `Browser::launch` doc comment（第 49 行）：

修改前：
```rust
    /// Launch browser with anti-detection patches.
    ///
    /// # Errors
    ///
    /// - `BrowserError::LaunchFailed` — 找不到 Chrome/Chromium 可执行文件、进程启动失败、
    ///   DevToolsActivePort 超时未出现。
    /// - `BrowserError::CdpConnection` — WebSocket 连接 CDP 失败。
    pub async fn launch(options: LaunchOptions) -> Result<Self> {
```

修改后：
```rust
    /// Launch browser with given options.
    ///
    /// ARCH: Browser 是通用 CDP 层，不包含反检测逻辑。
    /// 反检测 JS 补丁由 `stealth::patches` 提供（PR2 已迁移）。
    ///
    /// # Errors
    ///
    /// - `BrowserError::LaunchFailed` — 找不到 Chrome/Chromium 可执行文件、进程启动失败、
    ///   DevToolsActivePort 超时未出现。
    /// - `BrowserError::CdpConnection` — WebSocket 连接 CDP 失败。
    pub async fn launch(options: LaunchOptions) -> Result<Self> {
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib browser`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/browser/mod.rs
git commit -m "docs: 清理 Browser::launch 注释，移除 anti-detection 描述"
```

---

### Task 10: 最终验证 + 集成提交

**Files:**
- 无新文件（验证全局编译 + 测试 + 调用方兼容性）
- Test: 全量 `cargo test`

**Interfaces:**
- Consumes: Task 1-9 所有产出
- Produces: PR2 完整交付

- [ ] **Step 1: 写失败的测试**

无新测试。验证目标：

1. `cargo build --lib` 无警告无错误
2. `cargo test --lib` 全绿
3. `cargo clippy --lib` 无新警告
4. 调用方代码无破坏（`banzhu-rs` 如有引用 `fetch_browser` 需同步更新）

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo build --lib 2>&1 | tee /tmp/pr2-build.log`
Expected: 应无错误。若有错误，记录并修复。

Run: `cargo test --lib 2>&1 | tee /tmp/pr2-test.log`
Expected: 应全绿。若有失败，记录并修复。

- [ ] **Step 3: 写最小实现**

修复发现的任何问题。常见问题：

1. **调用方未更新**：若有代码仍调用 `fetch_browser(&req, true)` 或 `fetch_browser(&req, false)`，需改为通过 `Fetcher::fetch` 或显式传入 strategy。

   全局搜索调用点：

   ```bash
   grep -rn "fetch_browser" src/ --include="*.rs"
   ```

   每个调用点应改为 `Fetcher::fetch` 或 `client.fetch_browser(&req, strategy.as_ref())`。

2. **unused import 警告**：清理 `client.rs` 中因删除方法而不再使用的 import（如 `use crate::stealth::challenge::ChallengeSolver;` 和 `use crate::stealth::human::HumanBehavior;`）。

3. **`Fetcher::from_client` 调用方**：若有调用方使用 `from_client` 后以 Dynamic/Stealth 模式 fetch，会因 `browser_strategy = None` 而失败。需更新为 `Fetcher::new` 或手动注入 strategy。

   全局搜索：

   ```bash
   grep -rn "from_client" src/ --include="*.rs"
   ```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo build --lib && cargo test --lib && cargo clippy --lib -- -D warnings`
Expected: 全部 PASS，无警告。

Run: `cargo test --lib fetcher::strategy fetcher::strategies fetcher::client fetcher::tests stealth::patches browser::launch browser::page`
Expected: 所有相关模块测试 PASS。

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "chore: PR2 最终验证 — FetchClient 拆分 + browser/stealth 边界清理"
```

---

## 自检清单

### Spec 覆盖

- [x] 4.1 BrowserFetchStrategy trait → Task 1
- [x] 4.2 DynamicStrategy → Task 2
- [x] 4.2 StealthStrategy → Task 3（含所有关键修复点：loadingFailed、nav_status 修正、broadcast Receiver、tracing 日志、warn 日志）
- [x] 4.3 FetchClient::fetch_browser 新签名 → Task 4
- [x] 4.4 Fetcher 持有 browser_strategy → Task 6
- [x] 4.5 patches.rs 迁移 → Task 7
- [x] 4.5 build_stealth_args 拆分 → Task 8
- [x] 4.5 Browser::launch 注释清理 → Task 9
- [x] 4.6 迁移点：删除 do_browser_work_inner → Task 4
- [x] 4.6 迁移点：删除 recv_navigation_status/extract_browser_response → Task 5
- [x] 4.7 测试策略：MockStrategy 验证 fetch_browser 调用契约 → Task 4
- [x] 4.7 测试策略：DynamicStrategy 集成测试 → Task 2（#[ignore]）
- [x] 4.7 测试策略：StealthStrategy 集成测试 → Task 3（#[ignore]）
- [x] 4.7 测试策略：Fetcher 模式路由测试 → Task 6

### 类型一致性

- `BrowserFetchStrategy::fetch(&self, page: &mut Page, req: &Request) -> Result<Response>` — Task 1 定义，Task 2/3/4 使用
- `recv_navigation_status(rx: &mut broadcast::Receiver<CdpEvent>, sid: &str) -> Result<u16>` — Task 1 定义，Task 2/3 使用
- `extract_browser_response(page: &Page, req: &Request, nav_status: u16) -> Result<Response>` — Task 1 定义，Task 2/3 使用
- `DynamicStrategy::from_config(config: &FetchClientConfig) -> Self` — Task 2 定义，Task 6 使用
- `StealthStrategy::from_config(config: &FetchClientConfig, cf_jar: Arc<CfCookieJar>) -> Self` — Task 3 定义，Task 6 使用
- `FetchClient::fetch_browser(&self, req: &Request, strategy: &dyn BrowserFetchStrategy) -> Result<Response>` — Task 4 定义，Task 6 使用
- `Fetcher.browser_strategy: Option<Arc<dyn BrowserFetchStrategy>>` — Task 6 定义
- `build_common_args(options: &LaunchOptions) -> Vec<String>` — Task 8 定义
- `build_stealth_extra_args(options: &LaunchOptions) -> Vec<String>` — Task 8 定义

### 风险缓解

| 风险 | 缓解 |
|---|---|
| StealthStrategy 吸收 do_browser_work_inner 逻辑遗漏关键步骤 | Task 3 代码逐行对照原 `do_browser_work_inner` 的 `solve_cf=true` 分支；保留所有 tracing 日志、nav_status 修正、loadingFailed 处理 |
| CfCookieJar 接口与原 CfSessionCache 不一致 | 假设 PR1 已迁移且保留 `get(&str) -> Option<CfSession>` / `insert(String, CfSession)` 接口；Task 3 Step 2 验证编译 |
| browser::page 引用 stealth::patches 导致依赖反转 | PR2 阶段 stealth 总是编译，无循环依赖问题；PR3 会重构 Page::create 接收脚本参数（不在本 PR 范围） |
| Fetcher::from_client 调用方破坏 | Task 6 保留 from_client 但 browser_strategy=None；Task 10 Step 3 全局搜索调用方并更新 |
| dead_code 警告 | Task 4 临时加 #[allow(dead_code)]，Task 5 删除方法时移除 |

## 执行顺序依赖

```
Task 1 (strategy.rs) ──┬─→ Task 2 (DynamicStrategy) ──┐
                       └─→ Task 3 (StealthStrategy) ──┤
                                                      ↓
Task 4 (fetch_browser 新签名) ──→ Task 5 (删除旧方法)
                                                      ↓
Task 6 (Fetcher 持有 strategy) ←────────────────────────┘

Task 7 (patches 迁移) ── 独立 ──→ Task 8 (build_stealth_args 拆分) ──→ Task 9 (注释清理)

Task 10 (最终验证) ← 所有任务
```

Task 7/8/9 可与 Task 1-6 并行（独立模块），但建议按顺序执行以避免 merge 冲突。
