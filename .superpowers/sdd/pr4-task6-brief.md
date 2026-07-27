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

