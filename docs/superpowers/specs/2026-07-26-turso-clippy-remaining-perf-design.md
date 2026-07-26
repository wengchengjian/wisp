# Turso 替换 + Clippy 修复 + 剩余性能优化 Design

> **日期**: 2026-07-26
> **状态**: Approved（待用户审查 spec 文档）
> **前置**: 2026-07-26-async-concurrency-fix 已完成（H1-H5、M1-M4/M6/M7）

## Goal

完成 wisp 框架异步/并发性能审查报告中剩余的优化项：
1. 用异步原生 turso crate 替换 rusqlite，消除手动 `spawn_blocking` 管理
2. 修复 601 个 clippy 1.97 新增 lint 警告
3. 处理剩余 Medium（M8/M9/M10）和全部 Low（L1-L7）任务
4. 清理死代码 SessionPool

## Architecture

四个独立工作项并行推进，互不依赖：
- **存储层替换**：`SqliteStore` 内部从 `rusqlite + spawn_blocking` 改为 `turso` 原生 async，对外保持 `Store` trait 不变
- **Clippy 修复**：`cargo clippy --fix` 自动修复 + 手动补充 + code review
- **性能优化**：M8/M9/M10 + L1-L7 共 10 处局部优化
- **死代码清理**：删除未使用的 `session_pool.rs` 模块

## Tech Stack

Rust + Tokio + turso 0.7 + parking_lot + moka + async-trait + tracing

## Global Constraints

- **分支**：master 主分支直接开发，禁止 worktree/feature branch
- **兼容性**：不向后兼容，删除旧字段/方法不保留兼容层
- **测试基线**：现有测试（约 439 个）必须每步后全过
- **依赖**：仅引入 turso（替换 rusqlite），其他不新增
- **保留 `feature = "sqlite"`**：turso 在 sqlite feature gate 后面，与 MemoryStore/FileStore 并列关系不变
- **提交信息**：中文简短一行，格式 `perf(<scope>): <描述>` 或 `fix(<scope>): <描述>`
- **验证命令**：`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings`
- **工作目录**：`/home/weng/wisp`

## File Structure

| 工作项 | 文件 | 责任 |
|---|---|---|
| Turso 替换 | `src/storage/sqlite.rs`, `Cargo.toml` | SqliteStore 重写 + 依赖切换 |
| Turso 调用点 | `src/crawl/runner.rs`, `src/mcp/*`, `tests/*` | `open/open_in_memory` 加 `.await` |
| Clippy 修复 | 全项目 | 机械修复 + 手动补充 |
| M8 | `src/crawl/middleware/builtin.rs` | score_body 大 body spawn_blocking |
| M9 | `src/crawl/scheduling/scheduler.rs` | Mutex 改 parking_lot |
| M10 | `src/fetcher/client.rs` | println! 改 tracing::trace! |
| L1 | `src/crawl/middleware/builtin.rs` | UaRotationMiddleware 用 &'static str |
| L2 | `src/proxy.rs` | ProxyPool::next 返回 Arc<str> |
| L3 | `src/crawl/engine.rs` | cf_domain_locks 改 moka::Cache |
| L4 | `src/crawl/middleware/builtin.rs` | score_body 短路扫描 |
| L5 | `src/crawl/observability/events.rs` | EventListener 用 Arc<EngineEvent> |
| L6 | `src/crawl/engine.rs` | sanitize_url（设计权衡，跳过实施 — 见 6.6） |
| L7 | `src/browser/cdp.rs` | execute 用 to_writer 预分配 buffer |
| 死代码 | `src/crawl/runtime/session_pool.rs`, `src/crawl/runtime/mod.rs` | 删除整个模块 |

---

## 1. Turso 替换 rusqlite

### 1.1 依赖变更

```toml
# Cargo.toml
[dependencies]
# 删除：rusqlite = { version = "0.x", features = ["bundled"], optional = true }
turso = { version = "0.7", optional = true }

[features]
sqlite = ["dep:turso"]  # 保留 feature gate，底层换成 turso
```

### 1.2 SqliteStore 接口契约

```rust
// src/storage/sqlite.rs
use turso::{Builder, Database, Value as TursoValue};

pub struct SqliteStore {
    db: Database,  // turso::Database 内部管理连接池
}

impl SqliteStore {
    pub async fn open(path: &Path) -> Result<Self> {
        let db = Builder::new_local(path.to_str().ok_or(...)?)
            .build()
            .await
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso open: {e}"))))?;
        let store = Self { db };
        store.init_schema().await?;
        Ok(store)
    }

    pub async fn open_in_memory() -> Result<Self> {
        let db = Builder::new_local(":memory:")
            .build()
            .await
            .map_err(...)?;
        let store = Self { db };
        store.init_schema().await?;
        Ok(store)
    }

    async fn init_schema(&self) -> Result<()> {
        let conn = self.db.connect().map_err(...)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;").await.map_err(...)?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;").await.map_err(...)?;
        conn.execute_batch(super::migrations::SCHEMA_V1).await.map_err(...)?;
        Ok(())
    }
}

#[async_trait]
impl Store for SqliteStore {
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let conn = self.db.connect().map_err(...)?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
             VALUES (?1, ?2, ?3, NULL, ?4)",
            (namespace.to_string(), key.to_string(), value.to_vec(), now),
        )
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("turso set: {e}"))))?;
        Ok(())
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.db.connect().map_err(...)?;
        let mut rows = conn.query(
            "SELECT value FROM kv \
             WHERE namespace = ?1 AND key = ?2 \
               AND (ttl_secs IS NULL OR cached_at + ttl_secs >= CAST(strftime('%s','now') AS INTEGER))",
            (namespace.to_string(), key.to_string()),
        )
        .await
        .map_err(...)?;
        if let Some(row) = rows.next().await.map_err(...)? {
            let val = row.get_value(0).map_err(...)?;
            // turso Value::Blob → Vec<u8>
            match val {
                TursoValue::Blob(b) => Ok(Some(b)),
                TursoValue::Null => Ok(None),
                _ => Err(WispError::Storage(StorageError::General("expected blob".into()))),
            }
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let conn = self.db.connect().map_err(...)?;
        conn.execute(
            "DELETE FROM kv WHERE namespace = ?1 AND key = ?2",
            (namespace.to_string(), key.to_string()),
        )
        .await
        .map_err(...)?;
        Ok(())
    }

    async fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let conn = self.db.connect().map_err(...)?;
        let now = chrono::Utc::now().timestamp();
        let ttl_secs = ttl.map(|d| d.as_secs() as i64);
        conn.execute(
            "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (namespace.to_string(), key.to_string(), value.to_vec(), ttl_secs, now),
        )
        .await
        .map_err(...)?;
        Ok(())
    }
}
```

### 1.3 关键决策

1. **删除 `Arc<Mutex<Connection>>`**：turso 的 `Database` 内部管理连接池，`connect()` 返回独立 `Connection`，无需手动加锁
2. **删除 `spawn_blocking`**：turso 原生 async，不需要 blocking pool
3. **`db.connect()` 是同步方法**：turso 内部池化 conn，不需要 await
4. **保留 `init_schema`**：改 async，用 `conn.execute_batch(...).await`
5. **`#[cfg(feature = "sqlite")]` 全部保留**：与 MemoryStore/FileStore 并列关系不变
6. **旧 schema 检测逻辑保留**：`init_schema` 中检测旧三表的逻辑不变

### 1.4 调用点（需加 `.await`）

| 文件 | 改动 |
|---|---|
| `src/storage/sqlite.rs` tests | `make_store()` 改 `async fn` + `.await` |
| `tests/crawl_e2e_real_test.rs` | `SqliteStore::open(...).await` |
| `tests/crawl_cache_real_test.rs` | 同上 |
| `tests/run_inner_test.rs` | 同上 |
| `tests/crawl_checkpoint_test.rs` | 同上 |
| `src/crawl/runner.rs` | `EngineBuilder::build` 中 SqliteStore 构造加 `.await`（如果存在）|
| `src/mcp/*` | 如有 SqliteStore 引用，加 `.await` |

### 1.5 测试调整

- `test_sqlite_store_async_does_not_block_runtime` 测试可删除（turso 原生 async，不可能阻塞 runtime）
- `old_schema_detection_does_not_break_new_store` 测试中 `rusqlite::Connection::open` 改为 turso `Builder::new_local`
- 其他测试保持不变，只是 `make_store()` 改 async

---

## 2. Clippy 修复

### 2.1 修复策略

1. **自动修复阶段**：
   ```bash
   cargo clippy --fix --all-targets --all-features --allow-dirty --allow-no-vcs
   cargo fmt --all  # 修复后格式化
   ```
   自动修复覆盖：
   - `uninlined_format_args`（`format!("{}", x)` → `format!("{x}")`）
   - `duration_suboptimal_units`（`Duration::from_secs(3600)` → `Duration::from_hours(1)`）
   - `unnecessary_literal_bound`（`fn name(&self) -> &str` → `-> &'static str`）
   - `redundant_clone`、`needless_borrow` 等

2. **手动补充阶段**：
   - `type_complexity`：抽取 `type` 别名
     ```rust
     // 旧：Mutex<HashMap<(String, String), (Vec<u8>, Option<Instant>)>>
     type MockStoreData = HashMap<(String, String), (Vec<u8>, Option<Instant>)>;
     // 新：Mutex<MockStoreData>
     ```
   - 自动修复漏掉的边缘情况
   - 验证自动修复没引入语义变化

3. **Code Review 阶段**：用 `superpowers:requesting-code-review` skill 对整个项目做代码审查，重点：
   - 自动修复是否破坏格式字符串语义
   - `&'static str` 改造是否真的返回字面量
   - type alias 抽取后可读性

4. **验证**：
   ```bash
   cargo test --all-features  # 全过
   cargo clippy --all-targets --all-features -- -D warnings  # 0 警告
   ```

### 2.2 风险

- 自动修复可能误改（如 `format!("{}", url)` 改成 `format!("{url}"` 但 `url` 是 `&Url` 类型）→ 由 `cargo build` 捕获
- `unnecessary_literal_bound` 可能误判（trait 方法返回 `&str` 但实际不是字面量）→ 手动审查每个

---

## 3. M8 score_body spawn_blocking

### 3.1 现状

`DynamicUpgradeMiddleware::score_body` 中 aho-corasick 匹配 + HTML 解析是 CPU 密集型，大 body（>1MB）直接在 tokio worker 上执行会阻塞 runtime。

### 3.2 方案

```rust
const LARGE_BODY_THRESHOLD: usize = 1 << 20; // 1MB

async fn score_body(&self, body: Vec<u8>, ...) -> Result<u32> {
    if body.len() > LARGE_BODY_THRESHOLD {
        let matcher = self.matcher.clone();  // aho-corasick 是 Clone + Send
        tokio::task::spawn_blocking(move || {
            Self::score_body_sync(&body, &matcher, ...)
        })
        .await
        .map_err(|e| ...)?
    } else {
        Ok(Self::score_body_sync(&body, &self.matcher, ...))
    }
}

fn score_body_sync(body: &[u8], matcher: &AhoCorasick, ...) -> u32 {
    // 原有 aho-corasick + 解析逻辑
}
```

**关键决策**：阈值 1MB。小于阈值直接同步执行（避免 spawn 开销），大于阈值 spawn_blocking。

### 3.3 收益

避免大页面（如 5MB HTML）阻塞 tokio worker，P99 延迟 -40%（按 review.md 估计）。

---

## 4. M9 Scheduler Mutex 优化

### 4.1 现状

```rust
// scheduler.rs
struct Scheduler {
    heap: Arc<tokio::sync::Mutex<HeapInner>>,  // 问题：tokio::sync::Mutex 但内部无 await
    seen_exact: Arc<DashSet<String>>,
    // ...
}
```

`push`/`pop` 内部都是同步操作（`heap.push`/`heap.pop`/`seq += 1`），没有 await。`tokio::sync::Mutex` 比 `parking_lot::Mutex` 重，且会 yield 让其他 task 抢占。

### 4.2 方案

```rust
use parking_lot::Mutex as PlMutex;

struct Scheduler {
    heap: Arc<PlMutex<HeapInner>>,  // 改为同步 Mutex
    // seen_* 不变
}

pub async fn push(&self, req: Request) {
    let is_new = match self.strategy { ... };
    if is_new {
        let mut g = self.heap.lock();  // 同步 lock，不 yield
        let seq = g.seq;
        g.heap.push(PrioritizedRequest { req, seq });
        g.seq += 1;
    }
}

pub async fn pop(&self) -> Option<Request> {
    let mut g = self.heap.lock();
    g.heap.pop().map(|p| p.req)
}
```

### 4.3 关键决策

- **保持 `async fn` 签名**：API 不变，调用方无需修改
- **`parking_lot::Mutex` 不跨 await 持锁**：内部无 await，完全安全
- **`pending_urls`/`seen_urls` 同样改同步 lock**

### 4.4 收益

消除 async Mutex 的 yield 开销，heap 操作 O(log N) 同步完成，锁持有时间极短。

---

## 5. M10 println! 改 tracing

### 5.1 现状

`fetcher/client.rs::do_browser_work_inner` 中有 `println!("[timing] ...")`，同步 stdout I/O 在 async 路径，每请求 3 次。

### 5.2 方案

```rust
// 旧
println!("[timing] goto+recv: {:?} ({})", elapsed, url);

// 新
tracing::trace!(elapsed = ?elapsed, url = %sanitize_url(&url), "goto+recv timing");
```

**关键决策**：用 `trace` 级别（不是 `info` 或 `debug`），因为 timing 是调试用，不需要默认输出。

### 5.3 收益

消除 stdout 阻塞，trace 级别默认不输出，可按需开启。

---

## 6. L1-L7 Low 项

### 6.1 L1: UaRotationMiddleware

```rust
// 旧：Vec<String>，每请求 clone String
pub struct UaRotationMiddleware {
    agents: Vec<String>,
    // ...
}

// 新：Vec<&'static str>，无 clone
pub struct UaRotationMiddleware {
    agents: Vec<&'static str>,
    // ...
}
```

**调用点**：`self.agents[idx]` 直接返回 `&'static str`，无 clone。

### 6.2 L2: ProxyPool::next 返回 Arc<str>

```rust
// 旧：Option<String>，每次 clone String
pub fn next(&self) -> Option<String> { ... }

// 新：Option<Arc<str>>，clone Arc 廉价
pub fn next(&self) -> Option<Arc<str>> { ... }
```

**调用点**：`req.proxy = proxy.map(|p| p.to_string())` 或保持 `Arc<str>` 传递（视 Request::proxy 字段类型决定）。

### 6.3 L3: cf_domain_locks 改 moka::Cache

```rust
// 旧：DashMap<String, Arc<Mutex<()>>>，无上限
pub struct EngineShared {
    cf_domain_locks: Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    // ...
}

// 新：moka::Cache，max 1024，自动 LRU 淘汰
pub struct EngineShared {
    cf_domain_locks: Arc<moka::Cache<String, Arc<tokio::sync::Mutex<()>>>>,
    // ...
}
```

**调用点**：
```rust
// 旧
let lock = ctx.shared.cf_domain_locks
    .entry(domain.clone())
    .or_insert_with(|| Arc::new(Mutex::new(())))
    .clone();

// 新
let lock = ctx.shared.cf_domain_locks
    .get_with(domain.clone(), async { Arc::new(Mutex::new(())) })
    .await;
```

### 6.4 L4: score_body 短路扫描

```rust
// 旧：全量扫描
let script_count = script_matcher.find_iter(body).count();

// 新：短路，达到阈值即停
const SCRIPT_DENSITY_THRESHOLD: usize = 6;
let script_count = script_matcher
    .find_iter(body)
    .take(SCRIPT_DENSITY_THRESHOLD)
    .count();
// 后续判断：script_count >= SCRIPT_DENSITY_THRESHOLD 即满足弱信号
```

### 6.5 L5: EventListener 用 Arc<EngineEvent>

```rust
// 旧：每个 listener 拿一份 clone
pub type EventListener = Box<dyn Fn(EngineEvent) -> BoxFuture<'static, ()> + Send + Sync>;

impl EventBus {
    pub async fn emit(&self, event: EngineEvent) {
        for listener in &self.listeners {
            listener(event.clone()).await;  // N 次 clone
        }
    }
}

// 新：Arc 共享，无 clone
pub type EventListener = Box<dyn Fn(Arc<EngineEvent>) -> BoxFuture<'static, ()> + Send + Sync>;

impl EventBus {
    pub async fn emit(&self, event: EngineEvent) {
        let event = Arc::new(event);  // 1 次 Arc 分配
        for listener in &self.listeners {
            listener(Arc::clone(&event)).await;  // N 次 Arc::clone（廉价）
        }
    }
}
```

**调用点**（全部在 `src/crawl/observability/events.rs` 内）：
- `pub fn logging_listener() -> EventListener`（line 157）— 闭包签名改 `move |event: Arc<EngineEvent>|`
- `pub fn metrics_listener(metrics: Arc<Metrics>) -> EventListener`（line 187）— 同上
- 测试中 `bus.on(Arc::new(move |_event: EngineEvent| { ... }))`（line 281, 349）— 改为 `Arc<EngineEvent>`
- banzhu-rs 有独立的 EventBus 实现，不受 wisp L5 改动影响

**访问 event 字段**：`Arc<EngineEvent>` 自动 deref，`event.method` 无变化。如果 listener 内部需要 owned `EngineEvent`，用 `(*event).clone()`。

### 6.6 L6: sanitize_url（设计权衡，跳过实施）

**分析**：
- `sanitize_url` 每次调用都 `url::Url::parse` + `String::replace` 分配 String
- tracing 路径中 `url = %sanitize_url(&req.url)` 已是惰性（tracing 默认只在 subscriber 记录时计算）
- 非 tracing 路径（`CrawlEvent::Error { url: sanitize_url(...) }`）必须分配 String，因为 CrawlEvent::url 字段是 String
- 改 CrawlEvent 为 lazy/Cow 模式复杂度过高，收益不匹配

**决策**：跳过 L6 实施。脱敏是必要的（避免日志泄露凭据），每次分配 String 不可避免。

### 6.7 L7: CDP execute 用 to_writer 预分配 buffer

```rust
// 旧：每命令分配 String
let text = serde_json::to_string(&msg).expect("CDP msg always serializable");
writer.send(Message::Text(text.into())).await?;

// 新：复用 buffer
// CdpSession 增加字段
pub struct CdpSession {
    // ...
    write_buf: tokio::sync::Mutex<Vec<u8>>,  // 复用 buffer
}

// execute_with_session 中
let mut buf = self.write_buf.lock().await;
buf.clear();
serde_json::to_writer(&mut *buf, &msg).expect("CDP msg always serializable");
writer.send(Message::Binary(buf.clone().into())).await?;
// 或：writer.send(Message::Text(String::from_utf8_lossy(&buf).into_owned().into())).await?;
```

**关键决策**：
- `Message::Binary` vs `Message::Text`：CDP 协议要求 Text，但 turso 内部用 Binary 也接受。实际 Chrome CDP 同时支持 Text 和 Binary。
- `buf.clone()` 仍有一次 clone（因为 writer.send 需要 owned）。但比 `to_string` 分配新 String 更廉价（Vec<u8> clone 比 String parse 廉价）。
- **简化方案**：不引入 `write_buf` 字段，直接用 `to_writer` 到局部 `Vec::with_capacity(256)`：

```rust
let mut buf = Vec::with_capacity(256);
serde_json::to_writer(&mut buf, &msg).expect("CDP msg always serializable");
let text = String::from_utf8(buf).expect("CDP msg always UTF-8");
writer.send(Message::Text(text.into())).await?;
```

收益：避免 `to_string` 内部多次 realloc（预分配 256 字节），但不消除 String 分配。**收益较小，可跳过**。

**最终决策**：L7 用简化方案（预分配 Vec::with_capacity(256)），不引入共享 buffer 字段。

---

## 7. 删除死代码 SessionPool

### 7.1 验证

`SessionPool` 仅在以下文件出现：
- `src/crawl/runtime/session_pool.rs`（自身）
- `docs/*.md`（文档）

无任何代码引用，确认为死代码。

### 7.2 方案

- 删除整个 `src/crawl/runtime/session_pool.rs` 文件
- 检查 `src/crawl/runtime/mod.rs`：
  - 如果只有 `pub mod session_pool;` 一行，删除整个 `runtime/` 目录
  - 如果还有其他模块，只删除 `pub mod session_pool;` 行
- 检查 `src/crawl/mod.rs` 是否有 `pub mod runtime;` 声明，相应清理

### 7.3 风险

极低。无引用即无影响。

---

## 验证步骤

### 单元验证（每个工作项完成后）

```bash
cd /home/weng/wisp
cargo build --all-features                    # 编译通过
cargo test --all-features                     # 全量测试 PASS
cargo clippy --all-targets --all-features -- -D warnings  # 0 警告
```

### 集成验证（全部完成后）

```bash
cd /home/weng/banzhu-rs
cargo build  # path 依赖 wisp，验证集成编译通过
```

### 性能验证（可选）

- M8: 大 body（5MB HTML）score_body 不阻塞 worker
- M9: 高并发（50 worker）Scheduler push/pop 吞吐
- L1-L7: 各自微小优化，无独立验证

---

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| turso 0.7.0 API 不稳定（7 天前发布） | 备选 libsql crate；turso API 与 rusqlite 相似，迁移成本低 |
| turso 参数绑定语法与 rusqlite 不同 | 按 turso 文档调整，参考 Quick Start 示例 |
| Clippy 自动修复误改 | cargo build 验证 + 手动 code review |
| L5 改 Arc<EngineEvent> 影响所有 listener | listener 数量少（项目内 < 5 处），手动改造可控 |
| M9 parking_lot::Mutex 跨 await 持锁 | 内部无 await，完全安全 |

---

## 不在范围内

- M5 SessionPool BinaryHeap 重构（已确认是死代码，直接删除）
- 基准测试（criterion benches，可作为后续 PR）
- 生产级监控（metrics crate Histogram，可作为后续 PR）

---

## 审核清单

请审核以下要点：

- [ ] Turso 替换后 `#[cfg(feature = "sqlite")]` 保持不变
- [ ] Turso 参数绑定语法（`(ns.to_string(), key.to_string(), ...)` vs rusqlite `params![...]`）
- [ ] Turso `Value::Blob` → `Vec<u8>` 转换是否正确
- [ ] Clippy 自动修复后是否破坏格式字符串语义
- [ ] M8 阈值 1MB 是否合理（过大可能不触发，过小可能误判）
- [ ] M9 `parking_lot::Mutex` 在 async fn 中是否安全（内部无 await 即安全）
- [ ] L5 所有 listener 注册点是否能找到（搜索 `EventBus::on` 或类似）
- [ ] L6 跳过自动修复是否可接受（设计权衡）
- [ ] SessionPool 删除前再次确认无引用
