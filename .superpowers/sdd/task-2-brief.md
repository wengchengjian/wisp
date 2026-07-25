## Task 2: Engine 默认值 + bin/wisp.rs CLI + mcp 测试改动

**Files:**
- Modify: `src/crawl/runner.rs` (EngineBuilder::infra 默认值)
- Modify: `src/bin/wisp.rs:120-128` (CLI 默认 FileStore，--db 走 #[cfg(feature="sqlite")])
- Modify: `src/mcp/tools.rs:276` (测试用 MemoryStore)
- Modify: `src/mcp/mod.rs:265` (测试用 MemoryStore)

**Interfaces:**
- Consumes: `MemoryStore::default()`, `FileStore::default()` (来自 Task 1)
- Produces: `EngineBuilder::infra()` 默认注入 cache_store + checkpoint_store

- [ ] **Step 1: 更新 EngineBuilder::infra() 默认值（src/crawl/runner.rs）**

定位 `pub fn infra()` 函数（约 L78-95），修改 `cache_store` 和 `checkpoint_store` 默认值：

```rust
// 旧
cache_store: None,
checkpoint_store: None,

// 新
cache_store: Some(Arc::new(crate::storage::MemoryStore::default())),
checkpoint_store: Some(Arc::new(crate::storage::FileStore::default())),
```

注意：此时 `MemoryStore` 和 `FileStore` 都已实现 `Default` trait（Task 1 完成）。

- [ ] **Step 2: 修改 src/bin/wisp.rs L120-128**

```rust
// 旧
McpCmd::Serve { db } => {
    let store: Arc<dyn wisp::Store> = if db == ":memory:" {
        Arc::new(wisp::SqliteStore::open_in_memory()?)
    } else {
        Arc::new(wisp::SqliteStore::open(std::path::Path::new(&db))?)
    };
    wisp::mcp::serve(store).await?;
}

// 新（默认用 FileStore；sqlite feature 启用时支持 --db）
McpCmd::Serve { db } => {
    let store: Arc<dyn wisp::Store> = {
        #[cfg(feature = "sqlite")]
        {
            if db != ":memory:" && !db.is_empty() {
                Arc::new(wisp::SqliteStore::open(std::path::Path::new(&db))?)
            } else {
                Arc::new(wisp::FileStore::default())
            }
        }
        #[cfg(not(feature = "sqlite"))]
        {
            // sqlite feature 未启用时忽略 db 参数，使用 FileStore
            let _ = db;
            Arc::new(wisp::FileStore::default())
        }
    };
    wisp::mcp::serve(store).await?;
}
```

注意：此时 Cargo.toml 还没有 `sqlite` feature 定义，`#[cfg(feature = "sqlite")]` 会被 cargo 警告"unused feature"。Task 3 添加 feature 定义后警告消失。临时可加 `#[allow(unused_attributes)]`。

- [ ] **Step 3: 修改 src/mcp/tools.rs L276 测试**

```rust
// 旧
let store: Arc<dyn Store> = Arc::new(crate::storage::SqliteStore::open_in_memory().unwrap());
// 新
let store: Arc<dyn Store> = Arc::new(crate::storage::MemoryStore::default());
```

- [ ] **Step 4: 修改 src/mcp/mod.rs L265 测试**

同 Step 3。

- [ ] **Step 5: 编译验证**

```bash
cd /home/weng/wisp && cargo build
```
Expected: 编译通过。

- [ ] **Step 6: 运行 mcp 测试**

```bash
cd /home/weng/wisp && cargo test --lib mcp
```
Expected: mcp 测试通过。

- [ ] **Step 7: 运行端到端测试**

```bash
cd /home/weng/wisp && cargo test --test crawl_checkpoint_test
cd /home/weng/wisp && cargo test --test crawl_cache_real_test
```
Expected: 验证 Engine 默认值生效，checkpoint 持久化到 `./wisp-data/`。

- [ ] **Step 8: 检查 wisp-data 目录生成**

```bash
cd /home/weng/wisp && ls wisp-data/
```
Expected: 出现 `checkpoint/`、`element/`、`response/` 子目录（或部分）。

注意：测试运行后会在 `./wisp-data/` 留下数据，需要在 `.gitignore` 中添加该目录。

- [ ] **Step 9: 更新 .gitignore**

在 `.gitignore` 中追加：
```
wisp-data/
```

- [ ] **Step 10: Commit**

```bash
cd /home/weng/wisp && git add -A && git commit -m "feat(engine): 默认注入 MemoryStore + FileStore + CLI 适配"
```

---

