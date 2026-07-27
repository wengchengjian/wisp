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

