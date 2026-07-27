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
