# Task 2.5（plan 补丁）：迁移 tests/ 目录到自由函数 API

## 背景

Task 1 把 `Store` trait 的业务方法（`save_checkpoint`/`load_checkpoint`/`delete_checkpoint`/`save_element`/`load_element`/`save_response`/`load_response` 等）改写为 `crate::storage` 下的自由函数，并从 `SqliteStore` 上删除了这些方法。但原 plan 没有任何 task 负责迁移 `tests/` 目录下仍用旧 API 的集成测试，导致 `cargo test`（含集成测试）编译失败（19 处 E0599 + 11 处 SqliteStore::open_in_memory 调用）。

Task 2 reviewer 判定这是 plan 自身遗漏，用户已批准插入本 Task 2.5 修复。

## 目标

把 `tests/` 目录下 6 个测试文件改用新的自由函数 API + `MemoryStore::default()`，让 `cargo test`（无 feature）编译通过并全部测试通过。

## API 映射表（旧 → 新）

| 旧 API（已删除） | 新 API（自由函数） |
|---|---|
| `store.save_checkpoint(name, &blob, ts)` | `wisp::storage::save_checkpoint(&*store, name, &blob)` —— **第三个参数 `ts` 已删除** |
| `store.load_checkpoint(name)` | `wisp::storage::load_checkpoint(&*store, name)` |
| `store.delete_checkpoint(name)` | `wisp::storage::delete_checkpoint(&*store, name)` |
| `store.save_element(url, key, &row)` | `wisp::storage::save_element(&*store, url, key, &row)` |
| `store.load_element(url, key)` | `wisp::storage::load_element(&*store, url, key)` |
| `store.save_response(method, url, &cached)` | `wisp::storage::save_response(&*store, method, url, &cached)` |
| `store.load_response(method, url)` | `wisp::storage::load_response(&*store, method, url)` |
| `SqliteStore::open_in_memory().unwrap()` | `MemoryStore::default()`（默认策略，除非测试明确测 SqliteStore 特有行为） |

**解引用规则**：
- `let store = SqliteStore::open_in_memory().unwrap();` → `let store = MemoryStore::default();`（`store` 是 `MemoryStore`，传 `&store` 即可，因为 `&MemoryStore` 会自动转 `&dyn Store`）
- `let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());` → `let store: Arc<dyn Store> = Arc::new(MemoryStore::default());`（`store` 是 `Arc<dyn Store>`，传 `&*store` 解引用为 `&dyn Store`）

## 文件清单与迁移位置

### 1. `tests/crawl_checkpoint_test.rs`（7 处业务方法 + 4 处 SqliteStore）

**L4 import**:
```rust
// 旧
use wisp::storage::{Store, SqliteStore};
// 新（去掉 SqliteStore，因为不再用；保留 Store 用于 trait）
use wisp::storage::{Store, MemoryStore};
```

**L9, L43, L55, L79**（4 处）:
```rust
// 旧
let store = SqliteStore::open_in_memory().unwrap();
// 新
let store = MemoryStore::default();
```

**L22**:
```rust
// 旧
store.save_checkpoint("test-spider", &blob, state.saved_at.timestamp()).unwrap();
// 新（去掉第三个参数 ts）
wisp::storage::save_checkpoint(&store, "test-spider", &blob).unwrap();
```

**L24**:
```rust
// 旧
let loaded = store.load_checkpoint("test-spider").unwrap().expect("should be saved");
// 新
let loaded = wisp::storage::load_checkpoint(&store, "test-spider").unwrap().expect("should be saved");
```

**L46**:
```rust
// 旧
store.save_checkpoint("s2", &blob, 0).unwrap();
// 新
wisp::storage::save_checkpoint(&store, "s2", &blob).unwrap();
```

**L47**:
```rust
// 旧
assert!(store.load_checkpoint("s2").unwrap().is_some());
// 新
assert!(wisp::storage::load_checkpoint(&store, "s2").unwrap().is_some());
```

**L49**:
```rust
// 旧
store.delete_checkpoint("s2").unwrap();
// 新
wisp::storage::delete_checkpoint(&store, "s2").unwrap();
```

**L50**:
```rust
// 旧
assert!(store.load_checkpoint("s2").unwrap().is_none());
// 新
assert!(wisp::storage::load_checkpoint(&store, "s2").unwrap().is_none());
```

**L56**:
```rust
// 旧
assert!(store.load_checkpoint("nonexistent").unwrap().is_none());
// 新
assert!(wisp::storage::load_checkpoint(&store, "nonexistent").unwrap().is_none());
```

**L87**:
```rust
// 旧
store.save_checkpoint("test_spider", &blob, 0).unwrap();
// 新
wisp::storage::save_checkpoint(&store, "test_spider", &blob).unwrap();
```

**L90**:
```rust
// 旧
let loaded = store.load_checkpoint("test_spider").unwrap().unwrap();
// 新
let loaded = wisp::storage::load_checkpoint(&store, "test_spider").unwrap().unwrap();
```

### 2. `tests/crawl_cache_real_test.rs`（1 处业务方法 + 1 处 SqliteStore）

**L31**:
```rust
// 旧
let store = Arc::new(SqliteStore::open_in_memory().unwrap());
// 新
let store: Arc<dyn wisp::storage::Store> = Arc::new(MemoryStore::default());
```
（注意：需检查原代码 `store` 的类型注解。如果原是 `let store = Arc::new(...)`，需明确标注 `Arc<dyn wisp::storage::Store>` 或确保后续 `&*store` 能解引用为 `&dyn Store`）

**L45**:
```rust
// 旧
.load_response("GET", "https://httpbin.org/get")
// 新
wisp::storage::load_response(&*store, "GET", "https://httpbin.org/get")
```
（注意：需检查 L45 的上下文。如果是 `store.load_response(...)` 链式调用，改为 `wisp::storage::load_response(&*store, ...)`。如果是 `Arc<SqliteStore>::load_response`，同样改。）

### 3. `tests/crawl_e2e_real_test.rs`（1 处业务方法 + 1 处 SqliteStore）

**L436**:
```rust
// 旧
let store = Arc::new(SqliteStore::open_in_memory().unwrap());
// 新
let store: Arc<dyn wisp::storage::Store> = Arc::new(MemoryStore::default());
```

**L450**:
```rust
// 旧
.load_response("GET", "https://httpbin.org/get")
// 新
wisp::storage::load_response(&*store, "GET", "https://httpbin.org/get")
```

### 4. `tests/integration.rs`（1 处业务方法 + 2 处 SqliteStore）

**L150, L169**:
```rust
// 旧
let store = SqliteStore::open_in_memory().unwrap();
// 新
let store = MemoryStore::default();
```

**L188**:
```rust
// 旧
let saved = store.load_element(url, "product-title").unwrap().expect("snapshot should be saved");
// 新
let saved = wisp::storage::load_element(&store, url, "product-title").unwrap().expect("snapshot should be saved");
```

（注意：需检查 L150/L169 的 `store` 类型。如果 `store` 是 `SqliteStore`，改为 `MemoryStore` 后传 `&store` 即可。如果是 `Arc<dyn Store>`，传 `&*store`。）

### 5. `tests/adaptive_test.rs`（5 处业务方法 + 1 处 SqliteStore）

**L7**（在 helper 函数内）:
```rust
// 旧
SqliteStore::open_in_memory().unwrap()
// 新
MemoryStore::default()
```

**L42, L76**:
```rust
// 旧
store.save_element(url, key, &snapshot.to_row(0)).unwrap();
// 新
wisp::storage::save_element(&store, url, key, &snapshot.to_row(0)).unwrap();
```

**L45, L80**:
```rust
// 旧
let loaded = store.load_element(url, key).unwrap().unwrap();
// 新
let loaded = wisp::storage::load_element(&store, url, key).unwrap().unwrap();
```

**L98**:
```rust
// 旧
let row = store.load_element(url, "name-key").unwrap();
// 新
let row = wisp::storage::load_element(&store, url, "name-key").unwrap();
```

### 6. `tests/run_inner_test.rs`（2 处业务方法 + 2 处 SqliteStore）

**L183, L216**:
```rust
// 旧
let store = Arc::new(SqliteStore::open_in_memory().unwrap());
// 新
let store: Arc<dyn wisp::storage::Store> = Arc::new(MemoryStore::default());
```

**L203, L245**:
```rust
// 旧
let ckpt = store.load_checkpoint("ckpt-clear-test").unwrap();
// 新
let ckpt = wisp::storage::load_checkpoint(&*store, "ckpt-clear-test").unwrap();
```

## 步骤

- [ ] **Step 1**: 修改 `tests/crawl_checkpoint_test.rs`（按上述 L4/L9/L22/L24/L43/L46/L47/L49/L50/L55/L56/L79/L87/L90 修改）
- [ ] **Step 2**: 修改 `tests/crawl_cache_real_test.rs`（L31/L45）
- [ ] **Step 3**: 修改 `tests/crawl_e2e_real_test.rs`（L436/L450）
- [ ] **Step 4**: 修改 `tests/integration.rs`（L150/L169/L188 + 检查 import）
- [ ] **Step 5**: 修改 `tests/adaptive_test.rs`（L7/L42/L45/L76/L80/L98 + 检查 import）
- [ ] **Step 6**: 修改 `tests/run_inner_test.rs`（L183/L203/L216/L245 + 检查 import）
- [ ] **Step 7**: 编译验证 `cargo build --tests`（必须 0 错误）
- [ ] **Step 8**: 运行集成测试 `cargo test --tests`（除 #[ignore] 外必须通过）
- [ ] **Step 9**: 运行 lib 测试 `cargo test --lib`（确保无回归）
- [ ] **Step 10**: Commit `git add tests/ && git commit -m "test: 迁移 tests/ 到自由函数 API + MemoryStore"`

## 注意事项

1. **去掉 `ts` 参数**：旧 `save_checkpoint(name, &blob, ts)` 的第三个参数 `ts`（i64 timestamp）在新自由函数中已删除。所有调用都要去掉这个参数。
2. **解引用**：
   - `let store = MemoryStore::default();` → 传 `&store`
   - `let store: Arc<dyn Store> = Arc::new(MemoryStore::default());` → 传 `&*store`
3. **import 调整**：
   - 去掉 `use wisp::storage::SqliteStore;`（除非该文件还有其它 SqliteStore 用法）
   - 添加 `use wisp::storage::MemoryStore;`（如果该文件用了 MemoryStore）
   - 自由函数通过 `wisp::storage::save_checkpoint(...)` 全路径调用，无需额外 import
4. **不修改测试逻辑**：只改 API 调用方式，不改测试用例本身（不断言、不改测试函数名、不改测试意图）
5. **保留 `#[ignore]` 标记**：原有 `#[ignore = "requires network access"]` 等标记保留
6. **特殊处理**：如果某个测试明确测 SqliteStore 特有行为（如 SQL 查询、schema 验证），保留 SqliteStore，仅迁移业务方法调用。但根据初步检查，所有 6 个文件都是测 checkpoint/element/response 的 save/load 往返，不依赖 Sqlite 特性，可全部改用 MemoryStore。
7. **run_inner_test.rs 特别注意**：这个文件测 Engine 的 run_inner。Engine 内部有自己的 checkpoint_store（Task 2 已默认注入 FileStore）。如果测试想验证 checkpoint 被保存到外部 store，需要让 Engine 用测试提供的 store。检查 EngineBuilder 是否有 `.checkpoint_store(store)` 方法，如果有则用之；如果没有，可能需要调整测试逻辑。**如果遇到这种情况，停下来报告，不要强行修改**。
