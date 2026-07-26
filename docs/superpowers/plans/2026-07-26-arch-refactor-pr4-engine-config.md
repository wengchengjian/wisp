# PR4: Engine 配置聚合 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 Engine/EngineConfig/EngineShared 三结构的 26 个散落字段聚合到 Arc<EngineConfig>，简化配置访问

**Architecture:** EngineConfig 扩展到 12 字段（吸收 EngineBuilder 字段），Engine 持有 Arc<EngineConfig>，EngineBuilder 简化为 4 字段（config + cache_store + checkpoint_store + autoscale），删除 engine.rs 内部 EngineConfig 和 EngineShared 子结构

**Tech Stack:** Rust, Tokio, std::sync::Arc

## Global Constraints

- 变量命名 snake_case
- 代码注释用中文
- 提交信息用中文（一行）
- TDD：先写测试再写实现
- 保持现有测试全绿
- 不向后兼容，只考虑最优解
- PR1-PR3 已完成
- API 变更：engine.max_concurrent 字段访问改为 engine.config().max_concurrent
- autoscale 字段保留在 EngineBuilder（AutoscaledPool 是运行时组件，不能放入只读 config；spec 6.3 的"3 字段"近似为 4 字段）

---

## 当前状态分析

### 字段散落情况

| 结构 | 位置 | 字段数 | 字段列表 |
|---|---|---|---|
| `Engine` | runner.rs:28-56 | 15 | fetch_client, cache_store, max_concurrent, max_pages, max_refetch_rounds, checkpoint_store, checkpoint_interval, control, autoscale, running, fetch_mode, obey_robots, max_retries, download_delay, auto_rules |
| `EngineBuilder` | runner.rs:59-74 | 13 | fetch_client_config, max_concurrent, max_pages, max_refetch_rounds, cache_store, checkpoint_store, checkpoint_interval, autoscale, fetch_mode, obey_robots, max_retries, download_delay, auto_rules |
| `engine::EngineConfig` (pub(crate)) | engine.rs:46-59 | 7 | client, fetch_mode, max_concurrent, obey_robots, engine_max_pages, max_refetch_rounds, max_retries |
| `engine::EngineShared` (pub(crate)) | engine.rs:62-81 | 8 | sched, follow_tx, proxy_clients, control, work_notify, middleware_chain, rule_engine, cf_domain_locks |

### 目标状态

| 结构 | 字段数 | 字段列表 |
|---|---|---|
| `runner::EngineConfig` (pub) | 12 | max_concurrent, max_pages, max_errors, fetch_mode, obey_robots, max_retries, max_refetch_rounds, download_delay, checkpoint_interval, checkpoint_name, auto_rules, fetch_client_config |
| `Engine` | 7 | config, fetch_client, cache_store, checkpoint_store, control, autoscale, running |
| `EngineBuilder` | 4 | config, cache_store, checkpoint_store, autoscale |
| `engine::EngineContext` (pub(crate)) | 12 | config (Arc<runner::EngineConfig>), client, sched, follow_tx, proxy_clients, control, work_notify, middleware_chain, rule_engine, cf_domain_locks, state |
| `engine::EngineConfig` | 删除 | 合并入 runner::EngineConfig |
| `engine::EngineShared` | 删除 | 字段挪到 EngineContext 顶层 |

### 命名冲突处理

- Task 1 在 runner.rs 新增 `pub struct EngineConfig`，与 engine.rs 的 `pub(crate) EngineConfig` 路径不同（`crate::crawl::runner::EngineConfig` vs `crate::crawl::engine::EngineConfig`），可共存
- Task 5 删除 engine.rs 的 `EngineConfig`，命名冲突消除

---

## File Structure

| 文件 | 职责 | 改动类型 |
|---|---|---|
| `src/crawl/runner.rs` | 公开 EngineConfig / Engine / EngineBuilder | Modify |
| `src/crawl/engine.rs` | 内部 EngineContext / EngineState | Modify |
| `src/crawl/middleware/mod.rs` | CrawlContext 增加 config 字段 | Modify |
| `src/crawl/mod.rs` | re-export EngineConfig | Modify |
| `src/lib.rs` | 顶层 re-export EngineConfig | Modify |

---

## Task 1: 扩展 EngineConfig 到 12 字段

**Files:**
- Modify: `src/crawl/runner.rs:1-17`（新增 EngineConfig struct，紧挨 Engine 之前）
- Test: `src/crawl/runner.rs`（tests 模块新增测试）

**Interfaces:**
- Consumes: `crate::fetcher::FetchClientConfig`, `crate::fetcher::FetchMode`
- Produces: `pub struct EngineConfig`（12 字段 + Default），路径 `crate::crawl::runner::EngineConfig`

- [ ] **Step 1: 写失败的测试**

在 `src/crawl/runner.rs` 的 `#[cfg(test)] mod tests` 中追加测试（在现有 `test_follow_rx_drained_without_mutex` 之前）：

```rust
    /// PR4 Task 1：验证 EngineConfig::default() 返回正确默认值。
    #[test]
    fn test_engine_config_default_values() {
        let config = super::EngineConfig::default();
        assert_eq!(config.max_concurrent, 8);
        assert_eq!(config.max_pages, 1000);
        assert_eq!(config.max_errors, 1000);
        assert_eq!(config.fetch_mode, crate::fetcher::FetchMode::Auto);
        assert!(config.obey_robots);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.max_refetch_rounds, 5);
        assert_eq!(config.download_delay, Duration::ZERO);
        assert_eq!(config.checkpoint_interval, 100);
        assert!(config.checkpoint_name.is_none());
        assert!(config.auto_rules.is_empty());
    }

    /// PR4 Task 1：验证 EngineConfig clone 保留所有字段值。
    #[test]
    fn test_engine_config_clone_preserves_fields() {
        let mut config = super::EngineConfig::default();
        config.max_concurrent = 16;
        config.max_pages = 500;
        config.max_errors = 50;
        config.fetch_mode = crate::fetcher::FetchMode::Stealth;
        config.obey_robots = false;
        config.max_retries = 7;
        config.download_delay = Duration::from_millis(200);
        config.checkpoint_name = Some("test_spider".to_string());
        config.auto_rules.push(("example.com".to_string(), crate::fetcher::FetchMode::Http));

        let cloned = config.clone();
        assert_eq!(cloned.max_concurrent, 16);
        assert_eq!(cloned.max_pages, 500);
        assert_eq!(cloned.max_errors, 50);
        assert_eq!(cloned.fetch_mode, crate::fetcher::FetchMode::Stealth);
        assert!(!cloned.obey_robots);
        assert_eq!(cloned.max_retries, 7);
        assert_eq!(cloned.download_delay, Duration::from_millis(200));
        assert_eq!(cloned.checkpoint_name.as_deref(), Some("test_spider"));
        assert_eq!(cloned.auto_rules.len(), 1);
        assert_eq!(cloned.auto_rules[0].0, "example.com");
        assert_eq!(cloned.auto_rules[0].1, crate::fetcher::FetchMode::Http);
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib test_engine_config_default_values`
Expected: FAIL（`EngineConfig` 未定义）

- [ ] **Step 3: 写最小实现**

在 `src/crawl/runner.rs` 第 17 行（`use crate::fetcher::{FetchClient, FetchClientConfig};` 之后、`/// 爬虫引擎基础设施` 注释之前）插入：

```rust
/// 所有只读引擎配置聚合（Arc 共享，构建后不可变）。
///
/// ARCH: 替代散落在 Engine/EngineConfig/EngineShared 的 26 字段。
/// 构建后通过 Arc 共享，所有模块通过 `&Arc<EngineConfig>` 访问。
#[derive(Debug, Clone)]
pub struct EngineConfig {
    // === 并发与配额 ===
    /// 最大并发数。
    pub max_concurrent: usize,
    /// 最大爬取页数（引擎级兜底）。
    pub max_pages: usize,
    /// 最大错误数（达到此上限引擎停止）。
    pub max_errors: usize,

    // === 抓取行为 ===
    /// 抓取模式（Http/Dynamic/Stealth/Auto）。
    pub fetch_mode: crate::fetcher::FetchMode,
    /// 是否遵守 robots.txt。
    pub obey_robots: bool,
    /// 网络错误重试上限（fetch 失败后同步重试）。
    pub max_retries: u32,
    /// 响应中间件 Refetch 最大轮数。
    pub max_refetch_rounds: usize,
    /// 下载延迟（每次请求前的等待时间）。
    pub download_delay: Duration,

    // === 检查点 ===
    /// 检查点保存间隔（页数）。
    pub checkpoint_interval: usize,
    /// 检查点自定义名称（默认使用 spider name）。
    pub checkpoint_name: Option<String>,

    // === 自动模式规则 ===
    /// Auto 模式 URL 正则规则（优先级最高，跳过嗅探）。
    pub auto_rules: Vec<(String, crate::fetcher::FetchMode)>,

    // === HTTP 配置（含 proxy 子字段） ===
    /// FetchClient 配置（HTTP 连接池/超时/浏览器等基础设施配置）。
    pub fetch_client_config: crate::fetcher::FetchClientConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 8,
            max_pages: 1000,
            max_errors: 1000,
            fetch_mode: crate::fetcher::FetchMode::Auto,
            obey_robots: true,
            max_retries: 3,
            max_refetch_rounds: 5,
            download_delay: Duration::ZERO,
            checkpoint_interval: 100,
            checkpoint_name: None,
            auto_rules: Vec::new(),
            fetch_client_config: crate::fetcher::FetchClientConfig::default(),
        }
    }
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib test_engine_config_default_values test_engine_config_clone_preserves_fields`
Expected: PASS（2 个测试通过）

- [ ] **Step 5: 提交**

```bash
git add src/crawl/runner.rs
git commit -m "refactor: 新增公开 EngineConfig 聚合 12 字段"
```

---

## Task 2: Engine 持有 Arc<EngineConfig>

**Files:**
- Modify: `src/crawl/runner.rs:28-99`（Engine struct + Engine::infra + 新增 config() 方法）
- Modify: `src/crawl/runner.rs:655-677`（EngineBuilder::build 临时组装逻辑）
- Test: `src/crawl/runner.rs`（tests 模块新增测试）

**Interfaces:**
- Consumes: Task 1 的 `EngineConfig`
- Produces: `Engine::config() -> &Arc<EngineConfig>`，Engine 字段简化为 7 个

- [ ] **Step 1: 写失败的测试**

在 `src/crawl/runner.rs` 的 tests 模块中追加：

```rust
    /// PR4 Task 2：验证 Engine::config() 返回正确配置值。
    #[test]
    fn test_engine_config_accessor_returns_correct_values() {
        let engine = super::Engine::infra()
            .max_concurrent(16)
            .max_pages(500)
            .max_errors(50)
            .fetch_mode(crate::fetcher::FetchMode::Http)
            .obey_robots(false)
            .max_retries(5)
            .download_delay(Duration::from_millis(200))
            .build()
            .unwrap();

        assert_eq!(engine.config().max_concurrent, 16);
        assert_eq!(engine.config().max_pages, 500);
        assert_eq!(engine.config().max_errors, 50);
        assert_eq!(engine.config().fetch_mode, crate::fetcher::FetchMode::Http);
        assert!(!engine.config().obey_robots);
        assert_eq!(engine.config().max_retries, 5);
        assert_eq!(engine.config().download_delay, Duration::from_millis(200));
    }

    /// PR4 Task 2：验证 Engine::infra() 默认值正确。
    #[test]
    fn test_engine_infra_default_config() {
        let engine = super::Engine::infra().build().unwrap();
        let config = engine.config();
        assert_eq!(config.max_concurrent, 8);
        assert_eq!(config.max_pages, 1000);
        assert_eq!(config.fetch_mode, crate::fetcher::FetchMode::Auto);
        assert!(config.obey_robots);
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib test_engine_config_accessor_returns_correct_values`
Expected: FAIL（`Engine` 仍持散落字段，`config()` 方法未定义）

- [ ] **Step 3: 写最小实现**

**3a. 替换 Engine struct（runner.rs:28-56）：**

将原 Engine struct 定义替换为：

```rust
/// 爬虫引擎基础设施。长期持有，多次 run 不同 Spider。
///
/// PR4 重构：Engine 持有 Arc<EngineConfig>，所有只读配置通过 config() 访问。
/// - 不持有 Spider（删除 `spiders: Vec<Box<dyn Spider>>`）
/// - 共享：HTTP client / Store（SQLite 缓存 + checkpoint）
/// - 独立：每次 run 内部 Scheduler/去重/stats（per-Spider 隔离）
/// - 控制：per-Engine `EngineControl`
#[derive(Clone)]
pub struct Engine {
    /// 只读配置（Arc 共享给 Spider/Middleware）。
    pub(crate) config: Arc<EngineConfig>,
    /// 共享 FetchClient（HTTP 连接池 + BrowserPool，跨 Spider 复用）
    pub(crate) fetch_client: Arc<FetchClient>,
    pub(crate) cache_store: Option<Arc<dyn crate::storage::Store>>,
    pub(crate) checkpoint_store: Option<Arc<dyn crate::storage::Store>>,
    /// per-Engine 控制状态。
    pub(crate) control: Arc<control::EngineControl>,
    /// 自适应并发池（可选）。启用后 run_inner 动态调整并发数。
    pub(crate) autoscale: Option<Arc<crate::crawl::runtime::autoscale::AutoscaledPool>>,
    /// 运行时并发保护：防止同一 Engine 实例并发调用 run/run_stream。
    pub(crate) running: Arc<AtomicBool>,
}
```

**3b. 替换 Engine::infra()（runner.rs:82-99）和新增 config() 方法：**

在 `impl Engine {` 之后、`pub fn infra()` 之前插入 config() 方法，并修改 infra()：

```rust
impl Engine {
    /// 获取只读配置引用（Arc 共享给中间件和 Spider）。
    #[must_use]
    pub fn config(&self) -> &Arc<EngineConfig> {
        &self.config
    }

    /// 创建 Engine builder（纯基础设施构造器）。
    ///
    /// 替代原 `Engine::new(spider)` / `Engine::spiders(vec)` / `Engine::builder(spider)`。
    /// Engine 不再持有 Spider，长期持有共享底层资源。
    #[must_use]
    pub fn infra() -> EngineBuilder {
        EngineBuilder {
            config: EngineConfig::default(),
            cache_store: Some(Arc::new(crate::storage::MemoryStore::default())),
            checkpoint_store: Some(Arc::new(crate::storage::FileStore::default())),
            autoscale: None,
        }
    }
```

**3c. 临时更新 EngineBuilder::build()（runner.rs:655-677），把散落字段组装成 EngineConfig：**

将原 build() 替换为：

```rust
    /// 构建引擎实例。
    pub fn build(self) -> Result<Engine> {
        let fetch_client = Arc::new(FetchClient::new(self.fetch_client_config.clone())?);
        // PR4 Task 2 临时组装：从散落字段构造 EngineConfig。
        // Task 3 将 EngineBuilder 字段简化为 config + cache_store + checkpoint_store + autoscale 后，此处直接用 self.config。
        let config = EngineConfig {
            max_concurrent: self.max_concurrent,
            max_pages: self.max_pages,
            max_errors: 1000,
            fetch_mode: self.fetch_mode,
            obey_robots: self.obey_robots,
            max_retries: self.max_retries,
            max_refetch_rounds: self.max_refetch_rounds,
            download_delay: self.download_delay,
            checkpoint_interval: self.checkpoint_interval,
            checkpoint_name: None,
            auto_rules: self.auto_rules,
            fetch_client_config: self.fetch_client_config,
        };
        Ok(Engine {
            config: Arc::new(config),
            fetch_client,
            cache_store: self.cache_store,
            checkpoint_store: self.checkpoint_store,
            control: Arc::new(control::EngineControl::new()),
            autoscale: self.autoscale,
            running: Arc::new(AtomicBool::new(false)),
        })
    }
```

**3d. 临时禁用 run_inner 中对已删除字段的访问：**

run_inner 中 `self.max_concurrent` / `self.fetch_mode` / `self.obey_robots` / `self.max_pages` / `self.max_refetch_rounds` / `self.max_retries` / `self.download_delay` / `self.auto_rules` / `self.checkpoint_interval` 字段访问会编译失败（字段已删除）。临时改为 `self.config().xxx`：

- run_inner 第 213 行 `for (pattern, mode) in &self.auto_rules` → `for (pattern, mode) in &self.config().auto_rules`
- run_inner 第 217 行 `let fetch_mode = self.fetch_mode;` → `let fetch_mode = self.config().fetch_mode;`
- run_inner 第 218 行 `let max_concurrent = self.max_concurrent;` → `let max_concurrent = self.config().max_concurrent;`
- run_inner 第 220 行 `let obey_robots = self.obey_robots;` → `let obey_robots = self.config().obey_robots;`
- run_inner 第 274 行 `engine_max_pages: self.max_pages,` → `engine_max_pages: self.config().max_pages,`
- run_inner 第 275 行 `max_refetch_rounds: self.max_refetch_rounds,` → `max_refetch_rounds: self.config().max_refetch_rounds,`
- run_inner 第 276 行 `max_retries: self.max_retries,` → `max_retries: self.config().max_retries,`
- run_inner 第 290 行 `delay: self.download_delay,` → `delay: self.config().download_delay,`
- run_inner 第 298 行 `max_retries: self.max_retries,` → `max_retries: self.config().max_retries,`
- run_inner 第 468 行 `if pages_since_checkpoint >= self.checkpoint_interval {` → `if pages_since_checkpoint >= self.config().checkpoint_interval {`

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib test_engine_config_accessor_returns_correct_values test_engine_infra_default_config`
Expected: PASS

Run: `cargo build --lib`
Expected: 编译成功（run_inner 字段访问已同步更新）

- [ ] **Step 5: 提交**

```bash
git add src/crawl/runner.rs
git commit -m "refactor: Engine 持有 Arc<EngineConfig> 并提供 config() 方法"
```

---

## Task 3: EngineBuilder 简化为 4 字段

**Files:**
- Modify: `src/crawl/runner.rs:59-74`（EngineBuilder struct 简化）
- Modify: `src/crawl/runner.rs:536-677`（所有 setter 改为操作 self.config.xxx）
- Test: `src/crawl/runner.rs`（tests 模块新增测试）

**Interfaces:**
- Consumes: Task 2 的 `Engine` struct（已持 Arc<EngineConfig>）
- Produces: `EngineBuilder { config, cache_store, checkpoint_store, autoscale }`

- [ ] **Step 1: 写失败的测试**

在 `src/crawl/runner.rs` 的 tests 模块中追加：

```rust
    /// PR4 Task 3：验证 EngineBuilder 所有 setter 正确修改 config 字段。
    #[test]
    fn test_engine_builder_setters_modify_config() {
        let engine = super::Engine::infra()
            .max_concurrent(32)
            .max_pages(100)
            .max_errors(20)
            .fetch_mode(crate::fetcher::FetchMode::Stealth)
            .obey_robots(false)
            .max_retries(7)
            .max_refetch_rounds(10)
            .download_delay(Duration::from_millis(500))
            .checkpoint_name("custom_spider")
            .auto_rule("example.com", crate::fetcher::FetchMode::Http)
            .build()
            .unwrap();

        let config = engine.config();
        assert_eq!(config.max_concurrent, 32);
        assert_eq!(config.max_pages, 100);
        assert_eq!(config.max_errors, 20);
        assert_eq!(config.fetch_mode, crate::fetcher::FetchMode::Stealth);
        assert!(!config.obey_robots);
        assert_eq!(config.max_retries, 7);
        assert_eq!(config.max_refetch_rounds, 10);
        assert_eq!(config.download_delay, Duration::from_millis(500));
        assert_eq!(config.checkpoint_name.as_deref(), Some("custom_spider"));
        assert_eq!(config.auto_rules.len(), 1);
        assert_eq!(config.auto_rules[0].0, "example.com");
        assert_eq!(config.auto_rules[0].1, crate::fetcher::FetchMode::Http);
    }

    /// PR4 Task 3：验证 EngineBuilder 默认值通过 EngineConfig::default() 提供。
    #[test]
    fn test_engine_builder_default_values() {
        let engine = super::Engine::infra().build().unwrap();
        let config = engine.config();

        assert_eq!(config.max_concurrent, 8);
        assert_eq!(config.max_pages, 1000);
        assert_eq!(config.max_errors, 1000);
        assert_eq!(config.fetch_mode, crate::fetcher::FetchMode::Auto);
        assert!(config.obey_robots);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.max_refetch_rounds, 5);
        assert_eq!(config.download_delay, Duration::ZERO);
        assert_eq!(config.checkpoint_interval, 100);
        assert!(config.checkpoint_name.is_none());
        assert!(config.auto_rules.is_empty());
    }

    /// PR4 Task 3：验证 checkpoint setter 同时设置 store 和 interval。
    #[test]
    fn test_engine_builder_checkpoint_sets_store_and_interval() {
        use std::sync::Arc;
        let store = Arc::new(crate::storage::MemoryStore::default());
        let engine = super::Engine::infra()
            .checkpoint(Arc::clone(&store), 50)
            .build()
            .unwrap();
        assert_eq!(engine.config().checkpoint_interval, 50);
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib test_engine_builder_setters_modify_config`
Expected: FAIL（EngineBuilder 字段未简化，新 setter 如 max_errors / checkpoint_name 不存在）

- [ ] **Step 3: 写最小实现**

**3a. 替换 EngineBuilder struct（runner.rs:59-74）：**

```rust
/// Engine 构造器（Builder 模式）。
///
/// PR4 重构：字段简化为 4 个（config + cache_store + checkpoint_store + autoscale）。
/// 所有配置 setter 操作 self.config.xxx。
pub struct EngineBuilder {
    config: EngineConfig,
    cache_store: Option<Arc<dyn crate::storage::Store>>,
    checkpoint_store: Option<Arc<dyn crate::storage::Store>>,
    autoscale: Option<Arc<crate::crawl::runtime::autoscale::AutoscaledPool>>,
}
```

**3b. 替换所有 setter 方法（runner.rs:536-677）：**

```rust
impl EngineBuilder {
    /// 设置最大并发数。
    #[must_use]
    pub fn max_concurrent(mut self, n: usize) -> Self {
        self.config.max_concurrent = n;
        self
    }
    /// 设置最大爬取页数。
    #[must_use]
    pub fn max_pages(mut self, n: usize) -> Self {
        self.config.max_pages = n;
        self
    }
    /// 设置最大错误数（达到此上限引擎停止）。
    #[must_use]
    pub fn max_errors(mut self, n: usize) -> Self {
        self.config.max_errors = n;
        self
    }
    /// 设置 FetchClient 配置（HTTP 连接池/超时/浏览器等基础设施配置，跨 Spider 共享）。
    #[must_use]
    pub fn fetch_client_config(mut self, config: FetchClientConfig) -> Self {
        self.config.fetch_client_config = config;
        self
    }
    /// 设置代理（作用于共享 FetchClient 的所有 HTTP 请求）。
    #[must_use]
    pub fn proxy(mut self, proxy: &str) -> Self {
        self.config.fetch_client_config.proxy = Some(proxy.to_string());
        self
    }
    /// 设置中间件 Refetch 最大轮数（默认 5）。
    #[must_use]
    pub fn max_refetch_rounds(mut self, n: usize) -> Self {
        self.config.max_refetch_rounds = n;
        self
    }
    /// 设置响应缓存存储（注入 CacheMiddleware，永不过期）。
    /// 想要 TTL 的用户应通过 `Spider::middlewares()` 自定义 `CacheMiddleware`。
    pub fn cache_store(mut self, store: Arc<dyn crate::storage::Store>) -> Self {
        self.cache_store = Some(store);
        self
    }
    /// 设置检查点存储（定期保存爬取进度）。
    pub fn checkpoint(mut self, s: Arc<dyn crate::storage::Store>, interval: usize) -> Self {
        self.checkpoint_store = Some(s);
        self.config.checkpoint_interval = interval;
        self
    }
    /// 设置检查点自定义名称（默认使用 spider name）。
    #[must_use]
    pub fn checkpoint_name(mut self, name: impl Into<String>) -> Self {
        self.config.checkpoint_name = Some(name.into());
        self
    }

    /// 启用自适应并发池。min 为初始/下限，max 为上限。
    /// 启用后 run_inner 会启动后台 autoscaler，根据饱和度动态调整并发数。
    #[must_use]
    pub fn autoscale(mut self, min: usize, max: usize) -> Self {
        self.autoscale = Some(crate::crawl::runtime::autoscale::AutoscaledPool::new(
            min,
            max,
            crate::crawl::runtime::autoscale::AutoscaleConfig::default(),
        ));
        self
    }

    /// 同 autoscale(min, max) 但可自定义配置。
    #[must_use]
    pub fn autoscale_with_config(
        mut self,
        min: usize,
        max: usize,
        config: crate::crawl::runtime::autoscale::AutoscaleConfig,
    ) -> Self {
        self.autoscale = Some(crate::crawl::runtime::autoscale::AutoscaledPool::new(
            min, max, config,
        ));
        self
    }

    // === 引擎配置方法 ===

    /// 设置抓取模式（Http/Dynamic/Stealth/Auto，默认 Auto）。
    #[must_use]
    pub fn fetch_mode(mut self, mode: crate::fetcher::FetchMode) -> Self {
        self.config.fetch_mode = mode;
        self
    }

    /// 是否遵守 robots.txt（默认 true）。
    #[must_use]
    pub fn obey_robots(mut self, obey: bool) -> Self {
        self.config.obey_robots = obey;
        self
    }

    /// 设置网络错误重试上限（默认 3）。
    #[must_use]
    pub fn max_retries(mut self, n: u32) -> Self {
        self.config.max_retries = n;
        self
    }

    /// 设置下载延迟（默认 0，即无延迟）。
    #[must_use]
    pub fn download_delay(mut self, d: Duration) -> Self {
        self.config.download_delay = d;
        self
    }

    /// 设置下载延迟（毫秒）。
    #[must_use]
    pub fn download_delay_ms(mut self, ms: u64) -> Self {
        self.config.download_delay = Duration::from_millis(ms);
        self
    }

    /// Auto 模式：添加 URL 正则规则（优先级最高，跳过嗅探）。
    #[must_use]
    pub fn auto_rule(mut self, pattern: &str, mode: crate::fetcher::FetchMode) -> Self {
        self.config.auto_rules.push((pattern.to_string(), mode));
        self
    }

    /// 构建引擎实例。
    pub fn build(self) -> Result<Engine> {
        let fetch_client = Arc::new(FetchClient::new(self.config.fetch_client_config.clone())?);
        Ok(Engine {
            config: Arc::new(self.config),
            fetch_client,
            cache_store: self.cache_store,
            checkpoint_store: self.checkpoint_store,
            control: Arc::new(control::EngineControl::new()),
            autoscale: self.autoscale,
            running: Arc::new(AtomicBool::new(false)),
        })
    }
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib test_engine_builder_setters_modify_config test_engine_builder_default_values test_engine_builder_checkpoint_sets_store_and_interval`
Expected: PASS（3 个测试通过）

Run: `cargo test --lib`
Expected: 现有测试全绿

- [ ] **Step 5: 提交**

```bash
git add src/crawl/runner.rs
git commit -m "refactor: EngineBuilder 简化为 4 字段，setter 操作 config"
```

---

## Task 4: 删除 engine.rs 内部 EngineConfig，EngineContext 改用 Arc<runner::EngineConfig>

**Files:**
- Modify: `src/crawl/engine.rs:32-92`（删除 pub(crate) EngineConfig，EngineContext 字段重构）
- Modify: `src/crawl/engine.rs:370-493`（fetch_dispatch 中 ctx.config.client → ctx.client）
- Modify: `src/crawl/engine.rs:541-551`（build_crawl_context 中 ctx.config.xxx 访问）
- Modify: `src/crawl/engine.rs:893-1438`（4 个测试辅助函数更新）
- Modify: `src/crawl/runner.rs:268-321`（run_inner 创建 EngineContext 改用 Arc<runner::EngineConfig>）

**Interfaces:**
- Consumes: Task 1-3 的 `runner::EngineConfig`、Task 2 的 `Engine::config()`
- Produces: `EngineContext.config: Arc<runner::EngineConfig>`，`EngineContext.client: Arc<FetchClient>`

- [ ] **Step 1: 写失败的测试**

在 `src/crawl/engine.rs` 的 tests 模块中追加测试（验证 EngineContext 字段类型正确）：

```rust
    /// PR4 Task 4：验证 EngineContext.config 是 Arc<runner::EngineConfig>。
    #[test]
    fn test_engine_context_config_is_arc_runner_config() {
        let (ctx, _stats) = make_ctx();
        // 验证 config 字段类型为 Arc<crate::crawl::runner::EngineConfig>
        let _config: &crate::crawl::runner::EngineConfig = &ctx.config;
        // 验证 client 字段独立持有（不再嵌套在 config 中）
        let _client: &Arc<crate::fetcher::FetchClient> = &ctx.client;
        assert_eq!(ctx.config.fetch_mode, crate::fetcher::FetchMode::Http);
        assert_eq!(ctx.config.max_concurrent, 8);
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib test_engine_context_config_is_arc_runner_config`
Expected: FAIL（EngineContext.config 仍是 engine::EngineConfig，client 字段未独立）

- [ ] **Step 3: 写最小实现**

**3a. 替换 engine.rs 第 32-92 行（EngineContext + EngineConfig + EngineShared + EngineState）：**

```rust
// === EngineContext: 打包所有共享状态 ===

/// Engine 运行时上下文（单 Spider），由三层组成。
///
/// - `config`: 只读配置（Arc 共享，构建后不可变）
/// - `client`: 共享 FetchClient（跨 task 复用）
/// - `shared`: 跨 task 共享的可变状态
/// - `state`: per-run 可变状态
pub(crate) struct EngineContext {
    pub config: Arc<crate::crawl::runner::EngineConfig>,
    pub client: Arc<crate::fetcher::FetchClient>,
    pub shared: EngineShared,
    pub state: EngineState,
}

/// 跨 task 共享的可变状态。
pub(crate) struct EngineShared {
    pub sched: Arc<scheduler::Scheduler>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
    /// 代理 Client 缓存（key=proxy URL，避免每请求重建 Client）。
    pub proxy_clients: Arc<moka::sync::Cache<String, Arc<Client>>>,
    pub control: Arc<control::EngineControl>,
    pub work_notify: Arc<tokio::sync::Notify>,
    pub middleware_chain: Arc<middleware::MiddlewareChain>,
    pub rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
    /// 域名级 CF 挑战锁：防止初始并发请求全部走浏览器。
    pub cf_domain_locks: Arc<moka::sync::Cache<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// per-run 可变状态。
pub(crate) struct EngineState {
    pub spider: Arc<dyn Spider>,
    pub stats: Arc<SpiderStats>,
    pub items: Arc<Mutex<Vec<Value>>>,
    pub abort_flag: Arc<AtomicBool>,
    pub start: std::time::Instant,
    pub tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
    pub global_in_flight: Arc<AtomicUsize>,
}
```

注意：删除了原 `pub(crate) struct EngineConfig`（7 字段）。EngineShared 和 EngineState 暂时保留（Task 5 删除 EngineShared）。

**3b. 更新 fetch_dispatch 中 ctx.config.client 访问（engine.rs:379）：**

```rust
        match fetch_page(
            &ctx.client,  // 原 ctx.config.client
            &current_req,
            proxy.as_deref(),
            ctx.config.fetch_mode,
            &ctx.shared.rule_engine,
            &ctx.shared.proxy_clients,
            &ctx.shared.cf_domain_locks,
        )
        .await
```

**3c. 更新 build_crawl_context（engine.rs:541-551）：**

```rust
/// 从 EngineContext 构建中间件用的 CrawlContext 只读视图。
pub(crate) fn build_crawl_context(ctx: &EngineContext) -> middleware::CrawlContext {
    middleware::CrawlContext {
        spider_name: ctx.state.spider.name().to_string(),
        fetch_mode: ctx.config.fetch_mode,
        max_concurrent: ctx.config.max_concurrent,
        max_pages: ctx.config.max_pages,  // 原 engine_max_pages
        obey_robots: ctx.config.obey_robots,
        pages_crawled: ctx.state.stats.pages.load(Ordering::SeqCst),
        errors: ctx.state.stats.errors.load(Ordering::SeqCst),
    }
}
```

**3d. 更新 process_response 中 ctx.config.max_refetch_rounds 访问（engine.rs:224, 232, 236）：**

这些访问通过 Arc deref 仍可用，不需改动（字段名相同）。

**3e. 更新 runner.rs run_inner 创建 EngineContext（runner.rs:268-321）：**

```rust
        let ctx = Arc::new(engine::EngineContext {
            config: Arc::clone(self.config()),
            client: fetch_client.clone(),
            shared: engine::EngineShared {
                sched: sched.clone(),
                follow_tx,
                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                control: self.control.clone(),
                work_notify: Arc::new(tokio::sync::Notify::new()),
                middleware_chain: {
                    let defaults = middleware::builtin::default_middlewares(
                        middleware::builtin::DefaultMiddlewareConfig {
                            fetch_mode,
                            delay: self.config().download_delay,
                            obey_robots,
                            allowed_domains: spider.allowed_domains(),
                            max_depth,
                            cache_store: self.cache_store.clone(),
                            http_client: mw_http_client,
                            robots_cache: mw_robots_cache,
                            rule_engine: rule_engine.clone(),
                            max_retries: self.config().max_retries,
                        },
                    );
                    let mut chain = middleware::MiddlewareChain::new();
                    chain.middlewares = spider.middlewares();
                    chain.middlewares.extend(defaults);
                    chain.pipelines = spider.pipelines();
                    chain.sort();
                    Arc::new(chain)
                },
                rule_engine,
                cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
            },
            state: engine::EngineState {
                spider: spider.clone(),
                stats: stats.clone(),
                items,
                abort_flag: Arc::new(AtomicBool::new(false)),
                start: std::time::Instant::now(),
                tx,
                global_in_flight: Arc::new(AtomicUsize::new(0)),
            },
        });
```

注意：`engine_max_pages: self.config().max_pages` 已在 build_crawl_context 中处理（Task 3 改为 ctx.config.max_pages）。

**3f. 更新 4 个测试辅助函数（engine.rs:917-953, 1029-1073, 1139-1187, 1386-1438）：**

每个 make_ctx / make_ctx_with_retry / make_ctx_auto / make_ctx_with_tx 中，原 `config: EngineConfig { client, fetch_mode, ... }` 改为：

```rust
            config: Arc::new(crate::crawl::runner::EngineConfig {
                fetch_mode: FetchMode::Http,
                max_concurrent: 8,
                obey_robots: false,
                max_pages: 100,
                max_retries: 3,
                max_refetch_rounds: 5,
                ..Default::default()
            }),
            client: Arc::new(
                crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
                    .expect("build fetch client"),
            ),
            shared: EngineShared { ... },  // 保持不变
            state: EngineState { ... },  // 保持不变
```

对 `make_ctx_auto`，fetch_mode 改为 `FetchMode::Auto`，并设置 fetch_client_config：

```rust
            config: Arc::new(crate::crawl::runner::EngineConfig {
                fetch_mode: FetchMode::Auto,
                max_concurrent: 8,
                obey_robots: false,
                max_pages: 100,
                max_retries,
                max_refetch_rounds: 5,
                fetch_client_config: crate::fetcher::FetchClientConfig {
                    max_concurrent_pages: 0,
                    ..Default::default()
                },
                ..Default::default()
            }),
            client: Arc::new(
                crate::fetcher::FetchClient::new(
                    crate::crawl::runner::EngineConfig::default().fetch_client_config,
                ).expect("build fetch client"),
            ),
```

注意：make_ctx_auto 的 client 应使用 max_concurrent_pages=0 的 config 构建。简化为：

```rust
            client: {
                let fetch_config = crate::fetcher::FetchClientConfig {
                    max_concurrent_pages: 0,
                    ..Default::default()
                };
                Arc::new(crate::fetcher::FetchClient::new(fetch_config).expect("build fetch client"))
            },
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib test_engine_context_config_is_arc_runner_config`
Expected: PASS

Run: `cargo test --lib`
Expected: 现有测试全绿（4 个 make_ctx 测试函数更新后，所有依赖测试通过）

- [ ] **Step 5: 提交**

```bash
git add src/crawl/engine.rs src/crawl/runner.rs
git commit -m "refactor: EngineContext.config 改用 Arc<runner::EngineConfig>"
```

---

## Task 5: 删除 EngineShared，字段合并到 EngineContext 顶层

**Files:**
- Modify: `src/crawl/engine.rs:32-92`（删除 EngineShared，字段挪到 EngineContext）
- Modify: `src/crawl/engine.rs:97-551`（所有 ctx.shared.xxx → ctx.xxx）
- Modify: `src/crawl/engine.rs:893-1438`（4 个测试辅助函数更新）
- Modify: `src/crawl/runner.rs:268-321`（run_inner 创建 EngineContext 改为顶层字段）
- Modify: `src/crawl/runner.rs:323-532`（run_inner 中 ctx.shared.xxx → ctx.xxx）

**Interfaces:**
- Consumes: Task 4 的 `EngineContext`
- Produces: `EngineContext` 直接持有 sched/follow_tx/proxy_clients/control/work_notify/middleware_chain/rule_engine/cf_domain_locks（不再有 shared 子结构）

- [ ] **Step 1: 写失败的测试**

在 `src/crawl/engine.rs` 的 tests 模块中追加测试：

```rust
    /// PR4 Task 5：验证 EngineContext 直接持有 sched/control 等字段（无 shared 子结构）。
    #[test]
    fn test_engine_context_no_shared_substruct() {
        let (ctx, _stats) = make_ctx();
        // 直接访问 ctx.sched（原 ctx.shared.sched）
        let _sched: &Arc<scheduler::Scheduler> = &ctx.sched;
        // 直接访问 ctx.control
        let _control: &Arc<control::EngineControl> = &ctx.control;
        // 直接访问 ctx.work_notify
        let _notify: &Arc<tokio::sync::Notify> = &ctx.work_notify;
        // 直接访问 ctx.middleware_chain
        let _chain: &Arc<middleware::MiddlewareChain> = &ctx.middleware_chain;
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib test_engine_context_no_shared_substruct`
Expected: FAIL（`ctx.sched` 字段不存在，仍为 `ctx.shared.sched`）

- [ ] **Step 3: 写最小实现**

**3a. 替换 engine.rs 第 32-92 行（删除 EngineShared，字段挪到 EngineContext）：**

```rust
// === EngineContext: 打包所有共享状态 ===

/// Engine 运行时上下文（单 Spider）。
///
/// PR4 重构：删除 EngineShared 子结构，字段直接展开到 EngineContext 顶层。
/// 分为三组：
/// - 只读配置：config (Arc<runner::EngineConfig>) + client (Arc<FetchClient>)
/// - 跨 task 共享可变状态：sched / follow_tx / proxy_clients / control / work_notify / middleware_chain / rule_engine / cf_domain_locks
/// - per-run 可变状态：state
pub(crate) struct EngineContext {
    // === 只读配置 ===
    pub config: Arc<crate::crawl::runner::EngineConfig>,
    pub client: Arc<crate::fetcher::FetchClient>,

    // === 跨 task 共享可变状态（原 EngineShared） ===
    pub sched: Arc<scheduler::Scheduler>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
    /// 代理 Client 缓存（key=proxy URL，避免每请求重建 Client）。
    pub proxy_clients: Arc<moka::sync::Cache<String, Arc<Client>>>,
    pub control: Arc<control::EngineControl>,
    pub work_notify: Arc<tokio::sync::Notify>,
    pub middleware_chain: Arc<middleware::MiddlewareChain>,
    pub rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
    /// 域名级 CF 挑战锁：防止初始并发请求全部走浏览器。
    pub cf_domain_locks: Arc<moka::sync::Cache<String, Arc<tokio::sync::Mutex<()>>>>,

    // === per-run 可变状态 ===
    pub state: EngineState,
}

/// per-run 可变状态。
pub(crate) struct EngineState {
    pub spider: Arc<dyn Spider>,
    pub stats: Arc<SpiderStats>,
    pub items: Arc<Mutex<Vec<Value>>>,
    pub abort_flag: Arc<AtomicBool>,
    pub start: std::time::Instant,
    pub tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
    pub global_in_flight: Arc<AtomicUsize>,
}
```

**3b. 全局替换 ctx.shared → ctx（在 engine.rs 中）：**

所有以下访问需替换：
- `ctx.shared.sched` → `ctx.sched`
- `ctx.shared.follow_tx` → `ctx.follow_tx`
- `ctx.shared.proxy_clients` → `ctx.proxy_clients`
- `ctx.shared.control` → `ctx.control`
- `ctx.shared.work_notify` → `ctx.work_notify`
- `ctx.shared.middleware_chain` → `ctx.middleware_chain`
- `ctx.shared.rule_engine` → `ctx.rule_engine`
- `ctx.shared.cf_domain_locks` → `ctx.cf_domain_locks`

涉及行（engine.rs）：
- 第 99-107: `ctx.shared.control` → `ctx.control`（check_control_and_hook）
- 第 142-164: `ctx.shared.middleware_chain` → `ctx.middleware_chain`（process_request）
- 第 203-265: `ctx.shared.middleware_chain` → `ctx.middleware_chain`（process_response）
- 第 222-236: `ctx.config.max_refetch_rounds` 不变
- 第 246: `ctx.shared.rule_engine` / `ctx.shared.proxy_clients` / `ctx.shared.cf_domain_locks` → `ctx.rule_engine` / `ctx.proxy_clients` / `ctx.cf_domain_locks`
- 第 335-340: `ctx.shared.follow_tx` / `ctx.shared.work_notify` → `ctx.follow_tx` / `ctx.work_notify`
- 第 343-351: `ctx.state.tx` / `ctx.state.stats` 不变
- 第 372-493: `ctx.shared.middleware_chain` / `ctx.shared.rule_engine` / `ctx.shared.proxy_clients` / `ctx.shared.cf_domain_locks` / `ctx.config.max_retries` → 对应 `ctx.xxx`
- 第 410-417: `ctx.shared.rule_engine` → `ctx.rule_engine`
- 第 444: `ctx.state.spider` 不变
- 第 463-470: `ctx.state.tx` 不变

**3c. 更新 InFlightGuard 创建（engine.rs:441-448，runner.rs 中）：**

```rust
                        let ctx_c = ctx.clone();
                        let fut = async move {
                            let _g1 = engine::InFlightGuard {
                                counter: ctx_c.state.global_in_flight.clone(),
                                work_notify: ctx_c.work_notify.clone(),  // 原 ctx_c.shared.work_notify
                            };
                            let _g2 = engine::InFlightGuard {
                                counter: ctx_c.state.stats.in_flight.clone(),
                                work_notify: ctx_c.work_notify.clone(),  // 原 ctx_c.shared.work_notify
                            };
                            // ... 不变
                        };
```

**3d. 更新 runner.rs run_inner 创建 EngineContext（runner.rs:268-321）：**

```rust
        let ctx = Arc::new(engine::EngineContext {
            config: Arc::clone(self.config()),
            client: fetch_client.clone(),
            sched: sched.clone(),
            follow_tx,
            proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
            control: self.control.clone(),
            work_notify: Arc::new(tokio::sync::Notify::new()),
            middleware_chain: {
                let defaults = middleware::builtin::default_middlewares(
                    middleware::builtin::DefaultMiddlewareConfig {
                        fetch_mode,
                        delay: self.config().download_delay,
                        obey_robots,
                        allowed_domains: spider.allowed_domains(),
                        max_depth,
                        cache_store: self.cache_store.clone(),
                        http_client: mw_http_client,
                        robots_cache: mw_robots_cache,
                        rule_engine: rule_engine.clone(),
                        max_retries: self.config().max_retries,
                    },
                );
                let mut chain = middleware::MiddlewareChain::new();
                chain.middlewares = spider.middlewares();
                chain.middlewares.extend(defaults);
                chain.pipelines = spider.pipelines();
                chain.sort();
                Arc::new(chain)
            },
            rule_engine,
            cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
            state: engine::EngineState {
                spider: spider.clone(),
                stats: stats.clone(),
                items,
                abort_flag: Arc::new(AtomicBool::new(false)),
                start: std::time::Instant::now(),
                tx,
                global_in_flight: Arc::new(AtomicUsize::new(0)),
            },
        });
```

**3e. 更新 runner.rs run_inner 中 ctx.shared 访问（runner.rs:323-532）：**

- 第 324: `ctx.shared.middleware_chain` → `ctx.middleware_chain`
- 第 326-330: `ctx.shared.middleware_chain` → `ctx.middleware_chain`
- 第 337: `ctx.shared.work_notify` → `ctx.work_notify`（autoscale set_work_notify）
- 第 363-373: `ctx.shared.control` / `ctx.shared.sched` → `ctx.control` / `ctx.sched`
- 第 388: `ctx.shared.sched` → `ctx.sched`
- 第 417-424: `ctx.shared.work_notify` → `ctx.work_notify`
- 第 426-433: `ctx.shared.sched` / `ctx.shared.work_notify` → `ctx.sched` / `ctx.work_notify`
- 第 442-448: `ctx_c.shared.work_notify` → `ctx_c.work_notify`（InFlightGuard）
- 第 475: `ctx.state.stats` 不变

**3f. 更新 4 个测试辅助函数（engine.rs:917-953, 1029-1073, 1139-1187, 1386-1438）：**

每个 make_ctx / make_ctx_with_retry / make_ctx_auto / make_ctx_with_tx 中，原 `shared: EngineShared { ... }` 改为直接展开到 EngineContext 顶层：

```rust
        let ctx = EngineContext {
            config: Arc::new(crate::crawl::runner::EngineConfig {
                fetch_mode: FetchMode::Http,
                max_concurrent: 8,
                obey_robots: false,
                max_pages: 100,
                max_retries: 3,
                max_refetch_rounds: 5,
                ..Default::default()
            }),
            client: Arc::new(
                crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
                    .expect("build fetch client"),
            ),
            // 原 EngineShared 字段直接展开
            sched: Arc::new(scheduler::Scheduler::new()),
            follow_tx,
            proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
            control: Arc::new(control::EngineControl::new()),
            work_notify: Arc::new(tokio::sync::Notify::new()),
            middleware_chain: Arc::new(middleware::MiddlewareChain::new()),
            rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
            cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
            state: EngineState {
                spider: Arc::new(DummySpider) as Arc<dyn Spider>,
                stats: stats.clone(),
                items: Arc::new(Mutex::new(Vec::new())),
                abort_flag: Arc::new(AtomicBool::new(false)),
                start: Instant::now(),
                tx: None,
                global_in_flight: Arc::new(AtomicUsize::new(0)),
            },
        };
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib test_engine_context_no_shared_substruct`
Expected: PASS

Run: `cargo build --lib`
Expected: 编译成功

Run: `cargo test --lib`
Expected: 现有测试全绿

- [ ] **Step 5: 提交**

```bash
git add src/crawl/engine.rs src/crawl/runner.rs
git commit -m "refactor: 删除 EngineShared，字段合并到 EngineContext 顶层"
```

---

## Task 6: 中间件 CrawlContext 添加 config 字段

**Files:**
- Modify: `src/crawl/middleware/mod.rs:80-99`（CrawlContext 增加 config 字段）
- Modify: `src/crawl/engine.rs:541-551`（build_crawl_context 注入 config）
- Test: `src/crawl/middleware/mod.rs`（tests 模块新增测试）

**Interfaces:**
- Consumes: Task 4 的 `Arc<runner::EngineConfig>`
- Produces: `CrawlContext::config() -> &EngineConfig`，中间件可访问完整配置

- [ ] **Step 1: 写失败的测试**

在 `src/crawl/middleware/mod.rs` 末尾追加 tests 模块（如果不存在）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// PR4 Task 6：验证 CrawlContext 持有 config 字段并提供 config() 方法。
    #[test]
    fn test_crawl_context_has_config_field() {
        let config = std::sync::Arc::new(crate::crawl::runner::EngineConfig::default());
        let ctx = CrawlContext {
            spider_name: "test".to_string(),
            fetch_mode: crate::fetcher::FetchMode::Http,
            max_concurrent: 8,
            max_pages: 100,
            obey_robots: false,
            pages_crawled: 0,
            errors: 0,
            config: std::sync::Arc::clone(&config),
        };
        // 验证 config() 方法返回 &EngineConfig
        let cfg: &crate::crawl::runner::EngineConfig = ctx.config();
        assert_eq!(cfg.max_concurrent, 8);
        assert_eq!(cfg.max_pages, 1000);  // default
        assert_eq!(cfg.fetch_mode, crate::fetcher::FetchMode::Auto);  // default
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib test_crawl_context_has_config_field`
Expected: FAIL（CrawlContext 无 config 字段和 config() 方法）

- [ ] **Step 3: 写最小实现**

**3a. 修改 CrawlContext（middleware/mod.rs:80-99）：**

```rust
/// 引擎上下文只读视图（暴露给中间件）。
///
/// 中间件可读取引擎级配置和统计信息，用于决策（如根据已爬取页数调整策略）。
/// PR4 重构：新增 config 字段（Arc<EngineConfig>），中间件可通过 config() 访问完整配置。
#[derive(Debug, Clone)]
pub struct CrawlContext {
    /// Spider 名称
    pub spider_name: String,
    /// 当前抓取模式
    pub fetch_mode: FetchMode,
    /// 最大并发数
    pub max_concurrent: usize,
    /// 最大爬取页数
    pub max_pages: usize,
    /// 是否遵守 robots.txt
    pub obey_robots: bool,
    /// 已爬取页数（只读快照）
    pub pages_crawled: usize,
    /// 错误数（只读快照）
    pub errors: usize,
    /// 完整引擎配置（Arc 共享，中间件可访问任意配置字段）。
    pub config: std::sync::Arc<crate::crawl::runner::EngineConfig>,
}

impl CrawlContext {
    /// 获取完整引擎配置引用。
    #[must_use]
    pub fn config(&self) -> &crate::crawl::runner::EngineConfig {
        &self.config
    }
}
```

**3b. 修改 build_crawl_context（engine.rs:541-551）：**

```rust
/// 从 EngineContext 构建中间件用的 CrawlContext 只读视图。
pub(crate) fn build_crawl_context(ctx: &EngineContext) -> middleware::CrawlContext {
    middleware::CrawlContext {
        spider_name: ctx.state.spider.name().to_string(),
        fetch_mode: ctx.config.fetch_mode,
        max_concurrent: ctx.config.max_concurrent,
        max_pages: ctx.config.max_pages,
        obey_robots: ctx.config.obey_robots,
        pages_crawled: ctx.state.stats.pages.load(Ordering::SeqCst),
        errors: ctx.state.stats.errors.load(Ordering::SeqCst),
        config: Arc::clone(&ctx.config),
    }
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib test_crawl_context_has_config_field`
Expected: PASS

Run: `cargo test --lib`
Expected: 现有测试全绿

- [ ] **Step 5: 提交**

```bash
git add src/crawl/middleware/mod.rs src/crawl/engine.rs
git commit -m "refactor: CrawlContext 增加 config 字段，中间件可访问完整配置"
```

---

## Task 7: re-export EngineConfig + 文档注释更新

**Files:**
- Modify: `src/crawl/mod.rs:28`（re-export EngineConfig）
- Modify: `src/lib.rs`（顶层 re-export，如需）
- Test: 无新增测试，验证 cargo build 成功

**Interfaces:**
- Consumes: Task 1 的 `runner::EngineConfig`
- Produces: `wisp::crawl::EngineConfig` 公开路径

- [ ] **Step 1: 写失败的测试**

在 `src/crawl/mod.rs` 的 tests 模块中追加测试：

```rust
    /// PR4 Task 7：验证 wisp::crawl::EngineConfig 公开可访问。
    #[test]
    fn test_engine_config_public_accessible() {
        let config = crate::crawl::EngineConfig::default();
        assert_eq!(config.max_concurrent, 8);
        assert_eq!(config.max_pages, 1000);
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib test_engine_config_public_accessible`
Expected: FAIL（`crate::crawl::EngineConfig` 未 re-export）

- [ ] **Step 3: 写最小实现**

**3a. 修改 src/crawl/mod.rs 第 28 行：**

原：
```rust
pub use runner::{Engine, EngineBuilder};
```

改为：
```rust
pub use runner::{Engine, EngineBuilder, EngineConfig};
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib test_engine_config_public_accessible`
Expected: PASS

Run: `cargo build --lib`
Expected: 编译成功

- [ ] **Step 5: 提交**

```bash
git add src/crawl/mod.rs
git commit -m "refactor: re-export EngineConfig 到 wisp::crawl"
```

---

## Task 8: 最终验证 + banzhu-rs 兼容性检查

**Files:**
- Test: 无代码改动，仅运行验证命令

**Interfaces:**
- Consumes: Task 1-7 全部成果
- Produces: PR4 完整交付

- [ ] **Step 1: 写验证脚本（验证测试）**

在 `src/crawl/runner.rs` tests 模块中追加集成验证测试：

```rust
    /// PR4 Task 8：集成验证 — Engine 完整生命周期使用 config() 路径访问配置。
    #[tokio::test]
    async fn test_engine_full_lifecycle_with_config_accessor() {
        use futures::StreamExt;

        struct OkSpider;
        #[async_trait::async_trait]
        impl super::super::Spider for OkSpider {
            fn name(&self) -> &'static str { "ok" }
            fn start_urls(&self) -> Vec<String> { vec![] }
            async fn handle(&self, _resp: super::super::Response) -> (Vec<serde_json::Value>, Vec<super::super::Request>) {
                (vec![], vec![])
            }
        }

        let engine = super::Engine::infra()
            .max_concurrent(2)
            .max_pages(1)
            .max_errors(10)
            .fetch_mode(crate::fetcher::FetchMode::Http)
            .obey_robots(false)
            .download_delay_ms(10)
            .build()
            .unwrap();

        // 验证所有 config 字段通过 config() 访问
        assert_eq!(engine.config().max_concurrent, 2);
        assert_eq!(engine.config().max_pages, 1);
        assert_eq!(engine.config().max_errors, 10);
        assert_eq!(engine.config().fetch_mode, crate::fetcher::FetchMode::Http);
        assert!(!engine.config().obey_robots);
        assert_eq!(engine.config().download_delay, Duration::from_millis(10));

        // 验证 run_stream 不 panic
        let mut stream = engine.run_stream(OkSpider).events();
        let mut got_done = false;
        while let Some(event) = stream.next().await {
            if matches!(event, super::super::CrawlEvent::Done(_)) {
                got_done = true;
                break;
            }
        }
        assert!(got_done, "应收到 Done 事件");
    }
```

- [ ] **Step 2: 运行测试验证（应为通过状态，因 Task 1-7 已完成）**

Run: `cargo test --lib test_engine_full_lifecycle_with_config_accessor`
Expected: PASS

- [ ] **Step 3: 运行完整测试套件验证全绿**

Run: `cargo build --lib`
Expected: 编译成功，无警告（除预先存在的）

Run: `cargo test --lib`
Expected: 所有测试通过

Run: `cargo test --lib --features stealth`
Expected: 所有测试通过（验证 stealth feature 组合）

- [ ] **Step 4: 验证 banzhu-rs 兼容性**

Run: `cd /home/weng/banzhu-rs && cargo build`
Expected: 编译成功（banzhu-rs 只用 EngineBuilder setter 方法，不直接访问 engine 字段，PR4 保持 setter API 兼容）

Run: `cd /home/weng/banzhu-rs && cargo test --lib`
Expected: 现有测试通过

- [ ] **Step 5: 提交**

```bash
git add src/crawl/runner.rs
git commit -m "test: PR4 集成验证 Engine 配置聚合完整生命周期"
```

---

## 自检清单

### Spec 覆盖

- ✅ spec 6.2 EngineConfig 12 字段 → Task 1
- ✅ spec 6.2 Engine 7 字段 → Task 2
- ✅ spec 6.3 EngineBuilder 简化 → Task 3
- ✅ spec 6.4 删除 EngineShared → Task 5
- ✅ spec 6.4 engine.config().xxx 访问 → Task 2 + Task 4
- ✅ spec 6.4 Middleware 通过 ctx.config() 注入 &Arc<EngineConfig> → Task 6
- ✅ spec 6.5 测试策略 → Task 1-8 全部 TDD
- ✅ spec 6.6 风险缓解（中间件访问 / 字段访问统一 / 12 字段分组注释）→ 全部覆盖

### 类型一致性

- `EngineConfig`（runner.rs，pub）— Task 1 定义，Task 2-8 使用
- `Engine.config: Arc<EngineConfig>` — Task 2 定义
- `Engine::config() -> &Arc<EngineConfig>` — Task 2 定义
- `EngineBuilder.config: EngineConfig` — Task 3 定义
- `EngineContext.config: Arc<runner::EngineConfig>` — Task 4 定义
- `EngineContext.client: Arc<FetchClient>` — Task 4 定义（从原 engine::EngineConfig.client 挪出）
- `CrawlContext.config: Arc<EngineConfig>` — Task 6 定义
- `CrawlContext::config() -> &EngineConfig` — Task 6 定义

### 已知偏离

- **EngineBuilder 4 字段（非 spec 6.3 的 3 字段）**：autoscale 是运行时组件（AutoscaledPool 含状态），不适合放入只读 EngineConfig。保留独立字段。spec 6.3 的"3 字段"是近似值。
- **EngineContext 12 字段（非删除 EngineShared 后的 8 字段）**：EngineContext 直接持有 config + client + 8 个原 EngineShared 字段 + state，共 12 字段（含 state 子结构）。spec 6.4 的"删除 EngineShared（合并入 EngineConfig 或 Engine）"理解为合并到 EngineContext 顶层。

---

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| Task 4 涉及 4 个测试辅助函数更新，遗漏会导致编译失败 | Task 4 Step 3f 明确列出所有 4 个函数的更新模式 |
| Task 5 全局替换 ctx.shared → ctx 可能遗漏 | Task 5 Step 3b 列出所有需替换的访问点和行号 |
| banzhu-rs 可能直接访问 engine 字段 | Task 8 Step 4 验证 banzhu-rs 编译；预先检查确认 banzhu-rs 只用 setter（已确认） |
| EngineConfig 12 字段可读性 | 按功能分组注释（并发/抓取/检查点/规则/HTTP） |
| checkpoint_name 字段当前未在 run_inner 中使用 | Task 1 添加字段；后续 PR 可启用（spider_name 优先用 checkpoint_name） |
| max_errors 字段当前未在 run_inner 中使用 | Task 1 添加字段；后续 PR 可启用引擎级错误上限 |

---

## 执行顺序

```
Task 1 (扩展 EngineConfig) → 验证: cargo test test_engine_config_default_values
Task 2 (Engine 持有 Arc<EngineConfig>) → 验证: cargo test test_engine_config_accessor
Task 3 (EngineBuilder 简化) → 验证: cargo test test_engine_builder_setters
Task 4 (删除 engine::EngineConfig) → 验证: cargo test --lib
Task 5 (删除 EngineShared) → 验证: cargo test --lib
Task 6 (CrawlContext 增加 config) → 验证: cargo test test_crawl_context_has_config
Task 7 (re-export EngineConfig) → 验证: cargo test test_engine_config_public_accessible
Task 8 (最终验证) → 验证: cargo test --lib + banzhu-rs cargo build
```

每个 Task 独立可提交，建议按顺序执行（Task 2 依赖 Task 1，Task 4 依赖 Task 2-3，Task 5 依赖 Task 4）。
