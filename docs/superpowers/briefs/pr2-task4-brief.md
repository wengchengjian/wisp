# PR2 Task 4: FetchClient::fetch_browser 新签名 + 删除 do_browser_work_inner

**Files:**
- Modify: `src/fetcher/client.rs`（`fetch_browser` 签名改为接收 `&dyn BrowserFetchStrategy`；删除 `do_browser_work_inner`）
- Test: `src/fetcher/client.rs`（内联测试模块）

**Interfaces:**
- Consumes: `crate::fetcher::strategy::BrowserFetchStrategy`
- Produces: `pub async fn fetch_browser(&self, req: &Request, strategy: &dyn BrowserFetchStrategy) -> Result<Response>`

## 当前代码状态

`src/fetcher/client.rs` 当前：
- 第 19-20 行有 `use crate::stealth::challenge::ChallengeSolver;` 和 `use crate::stealth::human::HumanBehavior;`（仅被 `do_browser_work_inner` 使用）
- 第 163-188 行是 `fetch_browser(&self, req: &Request, solve_cf: bool)` 方法，内部调用 `self.do_browser_work_inner(...)`
- 第 190-369 行是 `do_browser_work_inner` 方法（约 180 行，需删除）
- 第 378-454 行是 `recv_navigation_status` 方法（Task 5 删除，本任务加 `#[allow(dead_code)]`）
- 第 457-484 行是 `extract_browser_response` 方法（Task 5 删除，本任务加 `#[allow(dead_code)]`）

## Step 1: 写失败的测试

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

## Step 2: 运行测试验证失败

Run: `cargo test --lib fetcher::client::tests::test_fetch_browser_no_pool_returns_error`
Expected: FAIL（`fetch_browser` 仍接收 `solve_cf: bool`，编译错误）

## Step 3: 写最小实现

修改 `src/fetcher/client.rs`：

### 3.1 在文件顶部添加 import

在 `use super::response::{Request, Response};` 后添加：

```rust
use super::strategy::BrowserFetchStrategy;
```

### 3.2 替换 `fetch_browser` 方法

将第 161-188 行的 `fetch_browser` 方法替换为：

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

### 3.3 删除 `do_browser_work_inner` 整个方法

删除从 `async fn do_browser_work_inner(` 到对应结束 `}` 的整个方法（约 180 行）。

### 3.4 给 `recv_navigation_status` 和 `extract_browser_response` 加 `#[allow(dead_code)]`

由于 `do_browser_work_inner` 已删除，这两个方法暂时无调用方，会触发 `dead_code` 警告。临时添加 `#[allow(dead_code)]` 注解（Task 5 会删除这两个方法）：

```rust
    #[allow(dead_code)]
    async fn recv_navigation_status(
        &self,
        rx: &mut tokio::sync::broadcast::Receiver<crate::browser::cdp::CdpEvent>,
        sid: &str,
    ) -> Result<u16> {
        // ... 保持原实现不变
    }

    #[allow(dead_code)]
    async fn extract_browser_response(
        &self,
        page: &crate::browser::page::Page,
        req: &Request,
        nav_status: u16,
    ) -> Result<Response> {
        // ... 保持原实现不变
    }
```

### 3.5 删除不再使用的 import

删除文件顶部以下两行（仅被 `do_browser_work_inner` 使用）：

```rust
use crate::stealth::challenge::ChallengeSolver;
use crate::stealth::human::HumanBehavior;
```

**注意**：保留 `recv_navigation_status` 和 `extract_browser_response` 的方法体不变（Task 5 才删除）。如果这两个方法使用了某些 import（如 `tokio::sync::broadcast`），暂时保留那些 import。

## Step 4: 运行测试验证通过

Run: `cargo test --lib fetcher::client`
Expected: PASS（`test_fetch_browser_no_pool_returns_error` 通过；`test_fetch_browser_invokes_strategy` 在无 Chrome 环境下因 acquire 失败而跳过断言）

Run: `cargo build --lib`
Expected: 无错误。可能有 `dead_code` 警告（如果 `#[allow(dead_code)]` 未生效），应无警告。

## Step 5: 提交

```bash
git add src/fetcher/client.rs
git commit -m "refactor: FetchClient::fetch_browser 接收 &dyn BrowserFetchStrategy"
```
