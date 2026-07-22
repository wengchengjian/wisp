# Task 9 Review: CrawlState + SQLite checkpoint 持久化

**Reviewer**: sub-agent (GLM-5.2)
**Commit**: 94734bc
**Diff range**: 0b4e731..94734bc
**日期**: 2026-07-21

---

## 一、Spec 合规评估

按 brief 12 个 Step 逐项核对：

| Brief 要求 | 状态 | 说明 |
|---|---|---|
| Step 1: 创建 `src/crawl/state.rs`，`CrawlState` 结构 + `new`/`from_stats`/`to_stats` | ✅ | 完全按 brief，字段/方法签名一致；`duration_ms: u128` 用毫秒往返 |
| Step 2: 在 `mod.rs` 声明 `pub mod state;` + `pub use state::CrawlState;` | ✅ | 在 `pub mod templates;` 之后正确追加 |
| Step 3: 给 `Method` / `SpiderRequest` 加 `Serialize, Deserialize` derive | ⚠️ | derive 已加；但 brief 未预见 `meta: serde_json::Value` 与 bincode 不兼容，implementer 加 `#[serde(skip)]` 修正（见 concerns 1） |
| Step 4: `Store` 加 `save_checkpoint` / `load_checkpoint` / `delete_checkpoint` | ✅ | 三个方法签名、SQL（INSERT OR REPLACE / SELECT / DELETE）、错误映射均一致；`crawl_checkpoints` 表 schema 在 `migrations.rs` 已存在 |
| Step 5: `Engine` 加 `checkpoint_store` + `checkpoint_interval` 字段 + `with_checkpoint` / `checkpoint_interval` builder | ✅ | 字段默认值 `None` / `100` 一致，builder 签名一致 |
| Step 6.1: 提取 `checkpoint_store` / `checkpoint_interval` / `spider_name` | ✅ | 在 `fetcher_config` 之后追加 |
| Step 6.2: checkpoint 恢复（load → deserialize → `tracing::info!` / `tracing::warn!`） | ✅ | 逻辑、日志格式与 brief 一致 |
| Step 6.3: seed URLs 分支（有 restored → push pending；无 → seed start_urls） | ✅ | 与 brief 完全一致（包括 stage 1 不恢复 seen_urls 的注释） |
| Step 6.4: stream 主循环加 `pages_since_checkpoint` 计数 + 定期保存 | ✅ | 计数、`CrawlState::from_stats` 快照、`bincode::serialize` + `save_checkpoint` 顺序与 brief 一致 |
| Step 6.5: `on_close` 之后加 `delete_checkpoint` 清理 | ✅ | 失败用 `tracing::warn!` 记录 |
| Step 7-8: `cargo check` / `cargo check --tests` 通过 | ✅ | report 已附，无新 warning |
| Step 9-10: 4 个测试 PASS | ✅ | `test result: ok. 4 passed` |
| Step 11: lib 测试 34 PASS | ✅ | `test result: ok. 34 passed` |
| Step 12: 提交 | ✅ | commit 94734bc |
| 全局约束：bincode 序列化为 blob 存入 SQLite | ✅ | 实现一致 |
| 全局约束：恢复 pending_urls 重新 seed | ✅ | 实现一致 |
| 全局约束：每 N 页保存（默认 100） | ✅ | 实现一致 |
| 全局约束：完成后删除 checkpoint | ✅ | 实现一致 |
| 全局约束：不破坏 Spider trait / SpiderRequest / CrawlStats 公开 API | ⚠️ | 字段类型和方法签名未变；但 `SpiderRequest.meta` 加 `#[serde(skip)]` 改变了 serde 行为（对所有 Serializer 生效，非仅 bincode），见 findings |

**合规结论**：所有功能要求满足，测试全通过。唯一偏离是 `meta` 的 `#[serde(skip)]`，是 brief 未预见的必要修正。

---

## 二、代码质量 Findings

### Critical
（无）

### Important

**I1. Implementer report 对 `#[serde(skip)]` 作用域的声明有误（事实性错误，代码本身无 bug）**

位置：`task-9-report.md` line 121-122

report 原文：
> 2. `#[serde(skip)]` 只影响 bincode 路径（checkpoint），不影响 `serde_json` 序列化语义（如果未来有 JSON 序列化 SpiderRequest 的场景，meta 仍正常序列化为 JSON 对象）。

**事实**：`#[serde(skip)]` 是 serde 通用属性，**对所有 `Serializer` 生效**，包括 `serde_json::Serializer` 和 `bincode::Serializer`。如果未来用 `serde_json::to_string(&spider_request)`，`meta` 字段会被**跳过**（序列化结果不含 `meta` 键），而非 report 声称的"正常序列化为 JSON 对象"。

**实际影响**：当前代码库中没有 `serde_json` 序列化 `SpiderRequest` 的场景（grep `\.meta\b` 仅命中 `with_meta` 的写入），所以**无功能 bug**。但 report 的声明会误导未来 maintainer 误判 `#[serde(skip)]` 的影响范围。

**建议**：不需要改代码；在 review 中纠正理解即可。如果未来 stage 2/3 引入 `serde_json` 序列化 `SpiderRequest` 的场景，需要重新评估（改用 `#[serde(with = "...")]` 将 `Value` 序列化为 JSON 字符串，或把 `meta` 字段类型改为 `Option<String>`）。

### Minor

**M1. `tests/crawl_checkpoint_test.rs` 第 3 行 `use std::sync::Arc;` 未使用**

位置：`tests/crawl_checkpoint_test.rs:3`

brief 原文 Step 9 包含这行 import，但 4 个测试均未使用 `Arc`。implementer 保留以"完全遵循 brief"，产生 1 个 warning。

**建议**：删除该行 import。brief 的原文是错的，不需要遵循 brief 的错误。

**M2. 定期保存失败静默忽略，建议加 `tracing::warn!`**

位置：`src/crawl/mod.rs:410-416`

```rust
if let Ok(blob) = bincode::serialize(&state) {
    let _ = store.save_checkpoint(
        &spider_name,
        &blob,
        state.saved_at.timestamp(),
    );
}
```

`bincode::serialize` 失败（`if let Ok` 不匹配）和 `save_checkpoint` 失败（`let _`）都被静默忽略。这是 brief 原文行为，符合 stage 1 best-effort 简化。但 checkpoint 失败可能导致断点续传失效，建议至少 `tracing::warn!` 记录，方便诊断。

**建议**（可选，stage 1 不强制）：
```rust
match bincode::serialize(&state) {
    Ok(blob) => {
        if let Err(e) = store.save_checkpoint(&spider_name, &blob, state.saved_at.timestamp()) {
            tracing::warn!("checkpoint 保存失败: {}", e);
        }
    }
    Err(e) => tracing::warn!("checkpoint 序列化失败: {}", e),
}
```

**M3. `Scheduler::restore` 方法存在但从未被调用（Task 8 遗留 dead code，非 Task 9 引入）**

位置：`src/crawl/scheduler.rs:106`

`Scheduler::restore(pending, seen)` 的 signature 完美匹配 checkpoint 恢复场景，但 Task 9 brief Step 6.3 用 `sched.push()` 循环而非 `restore()`。grep 验证 `sched\.restore` / `\.restore\(` 全库无调用。

**说明**：这不是 Task 9 引入的问题（`restore` 是 Task 8 留下的），brief 6.3 选择了 `push` 也是合理方案（因为 `seen_urls` 是 placeholder，恢复 seen 没意义）。仅作记录，**不需 fix**。

---

## 三、Concerns 评估（对 implementer 的 5 个 concerns 独立判断）

### Concern 1: `SpiderRequest.meta` 的 `#[serde(skip)]` 修复

**implementer 判断**：stage 1 可接受，未来 stage 2/3 改为 `#[serde(with = "...")]`。

**Reviewer 独立判断**：**同意 stage 1 可接受**，但 report 对作用域的声明有误（见 I1）。

依据：
- `meta` 字段确实只在 `with_meta` 中写入，从未被读取（grep `\.meta\b` 全库仅 1 处命中：`with_meta` 内的 `self.meta = meta`）
- `#[serde(skip)]` 反序列化时用 `Value::default()` = `Value::Null`，符合预期
- 不破坏 API（字段类型仍是 `Value`，`with_meta` 签名不变）
- bincode 1.x 不支持 `deserialize_any`，`serde_json::Value` 无法直接 bincode round-trip，`#[serde(skip)]` 是最小侵入修正
- 替代方案 `#[serde(with = "value_as_string")]` 会改变所有 serde 路径的序列化语义（meta 变成 JSON 字符串而非对象），且当前无 serde_json 序列化 SpiderRequest 的场景，不值得为未来假设引入复杂度

**结论**：保留 `#[serde(skip)]`，但需修正 report 中"只影响 bincode 路径"的错误声明。

### Concern 2: `tests/crawl_checkpoint_test.rs` 有未使用的 `use std::sync::Arc`

**implementer 判断**：保留以遵循 brief。

**Reviewer 独立判断**：**应删除**。brief 原文是错的，implementer 不应盲目遵循 brief 的错误。见 M1。

### Concern 3: checkpoint 保存是 best-effort

**implementer 判断**：符合 brief，stage 2 可改为 `tracing::warn!`。

**Reviewer 独立判断**：**同意 stage 1 可接受**，但建议现在就加 `tracing::warn!`（成本极低，诊断价值高）。见 M2。

### Concern 4: checkpoint_interval=100 默认值

**implementer 判断**：brief 指定，用户可通过 builder 调整。

**Reviewer 独立判断**：**同意**。brief 明确指定 100，是合理的 stage 1 默认值。小型爬取（< 100 页）不触发定期保存意味着无 checkpoint 残留，反而干净。用户可通过 `.checkpoint_interval(n)` 调整。

### Concern 5: seen_urls 不恢复（stage 1 placeholder）

**implementer 判断**：brief 接受的 stage 1 简化，恢复后已访问 URL 可能被重新爬取。

**Reviewer 独立判断**：**同意 stage 1 可接受**，但严重程度评估需细化：

**实际影响分析**：
1. **恢复时 pending 中的 URL**：通过 `sched.push()` 重新入队，`Scheduler::push` 会调用 `g.seen.insert(fp)` 自动 dedup（基于 URL fingerprint）。这部分**不会重复爬取**。
2. **已访问但已 pop 出去（不在 pending）的 URL**：`Scheduler::seen` 没恢复，所以如果 spider 在解析新页面时又发现这些 URL 并 `follow_tx.send()`，它们会被重新加入 pending 并重新爬取。这部分**会重复爬取**。
3. **`Scheduler::seen_urls()` 方法本身是 placeholder**（返回 `h.to_string()` 而非真实 URL），所以即使 `CrawlState::from_stats` 想恢复 seen，拿到的是 hash 字符串而非 URL，无法用于 dedup。

**严重程度**：Medium — 对于"断点续传"场景，意味着恢复后可能重复爬取已访问页面，浪费资源、可能触发站点反爬。但不会导致数据错误（爬虫结果具有幂等性，重复爬取只是浪费）。

**建议**：stage 1 接受；stage 2 应让 `Scheduler` 真正追踪 seen URLs（而非 hash），并在恢复时调用 `Scheduler::restore(pending, seen)` 而非 `push` 循环。

---

## 四、总评

### **APPROVED**

**理由**：
- 所有 brief spec 要求满足（12 个 Step 全部完成）
- 所有测试通过（4 个 checkpoint 测试 + 34 个 lib 测试）
- 唯一偏离（`meta` 的 `#[serde(skip)]`）是 brief 未预见的必要修正，stage 1 可接受
- 所有 5 个 concerns 的 stage 1 简化合理，implementer 的判断基本正确
- 无 Critical / Important 代码 bug（I1 是 report 声明错误，非代码 bug）

### Fix subagent 处理清单（均为 Minor，可选）

| # | Severity | Finding | 建议处理 |
|---|---|---|---|
| M1 | Minor | `tests/crawl_checkpoint_test.rs:3` `use std::sync::Arc;` 未使用 | **必做**：删除该行 |
| M2 | Minor | 定期保存失败静默忽略 | **可选**：加 `tracing::warn!`（成本极低） |

### 不需 fix 的项

- **I1**：report 声明错误，已在 review 中纠正，不需改代码
- **M3**：`Scheduler::restore` 是 Task 8 遗留 dead code，非 Task 9 范围
- **Concern 1-5**：stage 1 简化均合理，不需改代码

---

## 五、补充说明

### 关于 `SpiderRequest.meta` 的 `#[serde(skip)]` 的正确理解

| 序列化路径 | `#[serde(skip)]` 影响 | 实际后果 |
|---|---|---|
| `bincode::serialize` | 跳过 meta | checkpoint blob 不含 meta（预期） |
| `bincode::deserialize` | 用 `Value::Null` 填充 | 恢复后的 request meta = `Value::Null`（预期） |
| `serde_json::to_string` | 跳过 meta | JSON 不含 `meta` 字段（**report 声称"正常序列化为 JSON 对象"是错的**） |
| `serde_json::from_str` | 用 `Value::Null` 填充 | 反序列化后 meta = `Value::Null` |

当前代码库无 serde_json 序列化 SpiderRequest 的场景，故无实际 bug。但若未来引入此场景，需重新评估。

### 关于 `Scheduler::restore` 未被使用

`Scheduler::restore(pending, seen)` 方法在 Task 8 中定义，signature 完美匹配 checkpoint 恢复，但 Task 9 brief Step 6.3 选择了 `push()` 循环方案。两种方案在 stage 1（seen_urls 是 placeholder）下功能等价：

- `push()` 循环：重新 fingerprint + dedup，简单直接
- `restore()`：批量替换，但需要传 seen（placeholder 无意义）

stage 2 若真正追踪 seen URLs，应改用 `restore()` 以保留 pending 顺序并正确恢复 seen。
