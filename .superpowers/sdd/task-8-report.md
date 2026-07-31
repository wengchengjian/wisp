# Task 8 Report: 修复 RequestCache 键忽略 HTTP 方法

## 状态

DONE

## 提交 hash

```
cad9c82 fix(cache): RequestCache 键含 HTTP 方法
```

## 修改的文件清单

### 1. `src/crawl/runtime/request_cache.rs`（修改）

- 新增私有方法 `cache_key(method: &str, url: &str) -> String`，构造键 `"{method} {url}"`，与 dev_mode SQLite 缓存（按 `(url, method)` 存储）语义保持一致。
- `get` / `put` / `invalidate` 签名均新增 `method: &str` 参数，内部调用 `cache_key` 拼键。
- 4 个现有测试（`test_cache_put_and_get` / `test_cache_miss` / `test_cache_invalidate` / `test_cache_entry_count`）调用处补 `"GET"` 参数。
- 新增测试 `cache_key_includes_method`：存 GET 响应后断言 GET 命中、POST 不命中。

### 2. `src/crawl/engine.rs`（修改）

- `process_request` 内 `method_str` 定义从原 L161（RequestCache 查询之后、dev_mode SQLite 查询之前）上移到 L141（RequestCache 查询之前）。
- RequestCache 查询：`rc.get(method_str, &req.url)`（原 `rc.get(&req.url)`）。
- RequestCache 写入：`rc.put(method_str, &req.url, ...)`（原 `rc.put(&req.url, ...)`）。
- dev_mode SQLite 缓存段保持不变（已用 `(url, method_str)` 正确）。

### 3. `src/crawl/middleware/builtin.rs`（修改 — brief 漏列，必要补充）

brief 的 Files 与 Step 7 commit 命令均未提及此文件，但 `CacheMiddleware` 是 `RequestCache::{get,put}` 的第三个调用点，签名变更后必须同步适配，否则编译失败。

- `process_request`：计算 `method_str`（match `req.method`），`self.cache.get(method_str, &req.url)`。
- `process_response`：从 `resp.request.method` 计算 `method_str`，`self.cache.put(method_str, &resp.url, ...)`。
- import 新增 `Method`。

## TDD 验证（Red-Green）

由于工作树中实现已就绪，无法按 brief Step 1-2 顺序原生观察「先失败」，改为**回归式 TDD 验证**以确认测试确实捕获 bug：

1. **Red**：临时将 `cache_key` 改为忽略 `method`（仅返回 `url`），跑 `cargo test --lib crawl::runtime::request_cache::tests::cache_key_includes_method` → **FAILED**（POST 命中 GET 缓存，`post.is_none()` 断言失败）。符合预期。
2. **Green**：恢复 `format!("{} {}", method, url)` 实现，跑同一测试 → **PASS**。
3. **Cleanup**：确认恢复后的代码无残留实验代码。

## 测试结果

| 命令 | 结果 |
|------|------|
| `cargo build` | exit 0，编译通过（仅 7 个预先存在的 unused import warnings，与本次改动无关） |
| `cargo test --lib crawl::runtime::request_cache` | 5 passed; 0 failed（含新测试 + 现有 4 个） |
| `cargo test --lib` | 202 passed; 0 failed; 0 ignored |
| `cargo test --test unified_fetcher_test` | 11 passed; 0 failed; 1 ignored（网络测试） |

## 调用点核对

通过全仓搜索确认 `RequestCache::{get,put,invalidate}` 的所有调用点均已更新：

| 调用点 | 文件 | 状态 |
|--------|------|------|
| `RequestCache::get` / `put` | `src/crawl/engine.rs` (L151 查询 / L246 写入) | 已传 `method_str` |
| `RequestCache::get` / `put` | `src/crawl/middleware/builtin.rs` (CacheMiddleware L289 / L313) | 已传 `method_str` |
| `RequestCache::get` / `put` / `invalidate` | `src/crawl/runtime/request_cache.rs` 测试模块 | 已传 `"GET"` |
| 类型引用（无方法调用） | `src/crawl/runner.rs`、`src/crawl/mod.rs`、`src/crawl/runtime/mod.rs`、`src/lib.rs` | 无需修改 |
| tests/ 目录 | 无直接 RequestCache 调用 | 无需修改 |

## 关切点（concerns）

### 1. brief 漏列 `middleware/builtin.rs`（已处理，非阻塞）

brief 的「Files」段与 Step 7 commit 命令均只列 `request_cache.rs` + `engine.rs`，未提 `middleware/builtin.rs`。但 `CacheMiddleware` 同样调用 `RequestCache::{get,put}`，签名变更后必须适配。本次提交已包含该文件。提交信息亦补充说明此文件，保持 commit 自洽可编译。若严格按 brief Step 7 的 `git add` 命令只 stage 两个文件，会导致下次构建失败——故判定为必要偏离。

### 2. `CacheMiddleware` 与 engine 内置 RequestCache 查询存在重复（架构性，非本次范围）

engine 的 `process_request` 第 2 步直接查 `ctx.request_cache`，而 `CacheMiddleware::process_request` 也查同一个 `RequestCache`。若中间件链与 engine 内置查询同时启用，可能双重命中 / 双重写入。本次仅保证两者键语义一致（都用 `{method} {url}`），未重构去重——属架构性关切，建议后续 task 评估是否让 engine 完全委托给中间件层。

### 3. `method_str` 在 engine.rs 与 middleware 中重复 match（轻微重复）

`match req.method { Get => "GET", Post => "POST", ... }` 模式在 engine.rs（L142）与 middleware/builtin.rs（两处）重复。理想情况应抽到 `Method` 上的 `as_str()` 方法。但本次任务范围聚焦于 bug 修复，未做此重构以避免越界。

## next BASE

```
cad9c82
```

reviewer 可用 `git show cad9c82` 查看完整改动（3 files, +74/-31）。
