# Task 1 报告：Method::as_str() DRY 3 处字符串转换 (P1-5)

## 实现了什么

为 `src/crawl/mod.rs` 的 `Method` 枚举新增 `pub fn as_str(&self) -> &'static str`，返回标准大写 HTTP 动词（`"GET"`/`"POST"`/`"PUT"`/`"DELETE"`），并替换 3 处重复的 `match req.method { Method::Get => "GET", ... }` 转换：

1. `src/crawl/engine.rs:308-314` — `let method_str = match req.method { ... }` → `let method_str = req.method.as_str();`
2. `src/crawl/middleware/builtin.rs:283-288`（`CacheMiddleware::process_request`）— `match req.method { ... }` → `req.method.as_str()`
3. `src/crawl/middleware/builtin.rs:307-312`（`CacheMiddleware::process_response`）— `match resp.request.method { ... }` → `resp.request.method.as_str()`

新增测试 `crawl::tests::test_method_as_str_returns_standard_verbs`，断言 4 个变体的 `as_str()` 返回值。

## 修改的文件

- `src/crawl/mod.rs`：新增 `impl Method { pub fn as_str(&self) -> &'static str { ... } }`（line 55-65）+ 新增测试函数（line 508-514，位于现有 `#[cfg(test)] mod tests` 模块末尾，`}` 之前）。
- `src/crawl/engine.rs`：替换 1 处 method_str match（line 308-314 → 309）。
- `src/crawl/middleware/builtin.rs`：替换 2 处 method_str match；移除 `use crate::crawl::{..., Method}` 中的 `Method`（line 10），因替换后 builtin.rs 内已无未限定的 `Method` 引用（仅 line 451 `crate::crawl::Method::Get` 用全限定路径，不依赖该 import）。

## 执行的步骤（TDD：RED → GREEN → COMMIT）

### Step 1：写失败测试

在 `src/crawl/mod.rs` 现有 `#[cfg(test)] mod tests` 模块末尾（line 507 的 `}` 之前）追加：

```rust
    #[test]
    fn test_method_as_str_returns_standard_verbs() {
        assert_eq!(Method::Get.as_str(), "GET");
        assert_eq!(Method::Post.as_str(), "POST");
        assert_eq!(Method::Put.as_str(), "PUT");
        assert_eq!(Method::Delete.as_str(), "DELETE");
    }
```

> **对 brief 的微调**：brief Step 1 的代码片段未带 `#[test]` 属性。但 Step 6 的验证命令 `cargo test --lib crawl::tests::test_method_as_str_returns_standard_verbs` 期望测试被收集并 PASS——若无 `#[test]`，cargo 会报「0 tests matched」而非 PASS。且模块内现有测试均用 `#[test]`。故补上 `#[test]` 属性以匹配既有风格并满足 brief 自身的验证预期。这是满足 brief 验证命令所必需的最小调整。

### Step 2：RED — 验证测试失败

```
$ cargo test --lib crawl::tests::test_method_as_str_returns_standard_verbs
...
error[E0599]: no method named `as_str` found for enum `crawl::Method` in the current scope
   --> src/crawl/mod.rs:510:32
    |
 53 | pub enum Method { Get, Post, Put, Delete }
    | --------------- method `as_str` not found for this enum
...
error: could not compile `wisp` (lib test) due to 4 previous errors; 5 warnings emitted
```

RED 确认：编译失败，原因是 `Method` 上不存在 `as_str` 方法（4 个变体各 1 个错误），正是 brief Step 2 预期的失败原因。

### Step 3：实现 Method::as_str

在 `src/crawl/mod.rs:53` 的 `pub enum Method { ... }` 下方新增 impl 块（line 55-65），按 brief Step 3 逐字实现：

```rust
impl Method {
    /// 返回标准 HTTP 动词字符串（大写）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}
```

### Step 4：替换 engine.rs 的 method_str match

`src/crawl/engine.rs:308-314` 的 6 行 match 块替换为单行 `let method_str = req.method.as_str();`（注释保留）。`engine.rs:24` 的 `use ... Method` 仍被 line 720-730 的其他 match 块使用，无需调整。

### Step 5：替换 builtin.rs 两处 method_str match + 清理 import

- `builtin.rs:283-288` → `let method_str = req.method.as_str();`
- `builtin.rs:307-312` → `let method_str = resp.request.method.as_str();`

替换后 grep 确认 builtin.rs 内 `Method` 仅剩 line 451 的全限定路径 `crate::crawl::Method::Get`（不依赖 line 10 的 import）。按 brief 上下文提示，将 `use crate::crawl::{SpiderRequest, SpiderResponse, Method};` 改为 `use crate::crawl::{SpiderRequest, SpiderResponse};`，避免 `unused import: Method` 警告。

### Step 6：GREEN — 验证测试通过 + cargo build

```
$ cargo test --lib crawl::tests::test_method_as_str_returns_standard_verbs
...
running 1 test
test crawl::tests::test_method_as_str_returns_standard_verbs ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 206 filtered out; finished in 0.00s
```

GREEN 确认：目标测试 PASS；「206 filtered out」佐证基线测试数为 206，与 brief 预期一致。

```
$ cargo build
...
warning: unused imports: `AtomicBool`, `AtomicUsize`, and `Ordering` --> src/crawl/mod.rs:37:25
warning: unused import: `self`      --> src/crawl/mod.rs:42:23
warning: unused import: `tokio::sync::Mutex` --> src/crawl/mod.rs:43:5
warning: unused import: `Client`    --> src/crawl/mod.rs:46:25
warning: unused import: `self::stats::SpiderStats` --> src/crawl/mod.rs:49:5
warning: unused import: `StreamExt` --> src/crawl/mod.rs:42:29
warning: `wisp` (lib) generated 7 warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.31s
```

`cargo build` 通过。7 个 warning 全部是 mod.rs 中 **预存的** 未使用 import（与本次修改无关：`AtomicBool/AtomicUsize/Ordering`、`futures::stream::{self, StreamExt}`、`tokio::sync::Mutex`、`crate::http::Client`、`self::stats::SpiderStats`）。**无任何 `Method` 相关 warning，无 builtin.rs warning**——证明主动移除 `Method` import 的决策正确。

### 全量 lib 测试

```
$ cargo test --lib
...
test result: ok. 207 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
```

**207 passed; 0 failed** — 与 brief 预期完全吻合（206 基线 + 1 新增），无回归。

### Step 7：提交

```
$ git add src/crawl/mod.rs src/crawl/engine.rs src/crawl/middleware/builtin.rs
$ git commit -m "refactor: Method::as_str() DRY 3 处字符串转换 (P1-5)"
[master 6f5c44f] refactor: Method::as_str() DRY 3 处字符串转换 (P1-5)
 3 files changed, 24 insertions(+), 19 deletions(-)
```

仅暂存 brief 指定的 3 个文件（未用 `git add -A`/`git add .`）。commit 落在 `master` 分支，未切换分支、未建 worktree。

## 遇到的 warning 及处理

| Warning | 处理 |
|---|---|
| 潜在 `unused import: Method`（builtin.rs:10） | **主动移除** `Method` from `use crate::crawl::{SpiderRequest, SpiderResponse, Method};` → `use crate::crawl::{SpiderRequest, SpiderResponse};`。依据：替换两处 match 后 builtin.rs 内仅剩 line 451 的全限定路径 `crate::crawl::Method::Get`，不依赖该 import。最终 `cargo build` 无此 warning。 |
| 7 个预存 unused import（mod.rs:37/42/43/46/49） | **未处理**——均为本任务范围外的预存 warning，不属于本次重构引入。brief 明确「Only make changes directly required by the task」，故不顺手清理。 |

## 最终 commit SHA

```
6f5c44facefdb18be75b4a2142fa1c3101e63608
```

分支：`master`（项目强制 master-only）。

`git show --stat HEAD`：
```
 src/crawl/engine.rs             |  7 +------
 src/crawl/middleware/builtin.rs | 16 +++-------------
 src/crawl/mod.rs                | 20 ++++++++++++++++++++
 3 files changed, 24 insertions(+), 19 deletions(-)
```

## Self-review

1. **TDD 闭环完整**：先写测试 → 验证 RED（4 个 E0599 编译错误，原因精准为 `as_str` 不存在）→ 实现 → 验证 GREEN（1 passed）→ 验证全量（207 passed，无回归）→ 提交。每一步都跑了命令并观察输出，未跳过 RED 验证。

2. **替换完整性**：grep 确认 3 处 `match ... Method::Get => "GET", ...` 全部消除；engine.rs 与 builtin.rs 的 `method_str` 变量语义未变（仍为 `&'static str`，下游 `check_request_caches(ctx, &req, method_str)` / `self.cache.get(method_str, &req.url)` / `self.cache.put(method_str, ...)` 调用签名不变）。

3. **import 清理的边界**：仅移除 builtin.rs 的 `Method`；engine.rs 的 `Method` import 保留（line 720-730 仍用未限定 `Method::Get` 等）；mod.rs 不涉及（`Method` 在同文件内定义）。未顺手清理其他预存 warning，严守最小改动。

4. **brief 偏差已记录**：唯一与 brief 逐字片段的偏差是给测试加了 `#[test]` 属性。这是满足 brief 自身 Step 6 验证命令（期望 PASS 而非「0 tests matched」）所必需的最小修正，已在本报告 Step 1 说明。

5. **commit 卫生**：单 commit、单行 message 与 brief 完全一致；仅暂存 3 个指定文件；未触发 push；未改 git config；未创建文档/分支/worktree。

## 任何问题或顾虑

无。任务范围内全部步骤按 brief 完成，验证命令输出符合预期，无未决问题。
