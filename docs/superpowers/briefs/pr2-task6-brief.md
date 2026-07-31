# PR2 Task 6: Fetcher 持有 browser_strategy 字段

**Files:**
- Modify: `src/fetcher/mod.rs`（`Fetcher` 结构体 + `new` + `from_client` + `fetch`）
- Test: `src/fetcher/mod.rs`（内联测试模块）

**Interfaces:**
- Consumes: `crate::fetcher::strategy::BrowserFetchStrategy`、`crate::fetcher::strategies::{DynamicStrategy, StealthStrategy}`、`crate::cookie::CfCookieJar`
- Produces: `Fetcher { client, mode, browser_strategy: Option<Arc<dyn BrowserFetchStrategy>> }`

## 当前状态

Task 4 已临时修改 `Fetcher::fetch`，每次请求都构造新 strategy：
```rust
FetchMode::Dynamic => {
    let strategy = DynamicStrategy::from_config(self.client.config());
    self.client.fetch_browser(&req, &strategy).await
}
FetchMode::Stealth => {
    let config = self.client.config();
    let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
    let strategy = StealthStrategy::from_config(config, cf_jar);
    self.client.fetch_browser(&req, &strategy).await
}
```

本任务将 strategy 提升为 Fetcher 字段，构造一次复用多次。

**注意**：`src/crawl/engine.rs` 中的 `fetch_page_inner` 直接使用 `fetch_client.fetch_browser(...)`，不通过 Fetcher，本任务**不修改** engine.rs。

## Step 1: 写失败的测试

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

## Step 2: 运行测试验证失败

Run: `cargo test --lib fetcher::tests::test_fetcher_http_mode_has_no_strategy`
Expected: FAIL（`browser_strategy` 字段不存在，编译错误）

## Step 3: 写最小实现

### 3.1 添加 import

在 `src/fetcher/mod.rs` 的 `use` 区添加（如尚无）：

```rust
use crate::error::{Result, WispError};
use crate::fetcher::strategy::BrowserFetchStrategy;
```

注意：`CfCookieJar` 和 `DynamicStrategy`/`StealthStrategy` 已在 Task 4 中导入。如果未导入，添加：

```rust
use crate::cookie::CfCookieJar;
use crate::fetcher::strategies::{DynamicStrategy, StealthStrategy};
```

### 3.2 修改 `Fetcher` 结构体

添加 `browser_strategy` 字段：

```rust
pub struct Fetcher {
    client: Arc<FetchClient>,
    mode: FetchMode,
    /// 浏览器模式下的 strategy（Http/Auto 为 None）。
    /// ARCH: 由 Fetcher::new 根据 mode 自动构造。
    browser_strategy: Option<Arc<dyn BrowserFetchStrategy>>,
}
```

### 3.3 修改 `Fetcher::new`

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

### 3.4 修改 `Fetcher::from_client`

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

### 3.5 修改 `Fetcher::fetch`

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

### 3.6 添加 `browser_strategy` 访问器（可选，便于测试）

```rust
    /// 获取浏览器策略引用（如有）。
    #[must_use]
    pub fn browser_strategy(&self) -> Option<&Arc<dyn BrowserFetchStrategy>> {
        self.browser_strategy.as_ref()
    }
```

## Step 4: 运行测试验证通过

Run: `cargo test --lib fetcher`
Expected: PASS（4 个新测试 + 现有测试全通过）

Run: `cargo build --lib`
Expected: 无警告。

Run: `cargo test --lib`
Expected: 全量测试无回归。

## Step 5: 提交

```bash
git add src/fetcher/mod.rs
git commit -m "refactor: Fetcher 持有 browser_strategy，按 mode 自动构造"
```
