# Task 1 报告：补充 engine 诊断日志字段 + 删除示例 println

## 实现内容

### Step 1：engine.rs instrument 字段扩充（L173）

在 `process_response` 的 `#[tracing::instrument(...)]` 上新增两个字段：
- `body_len = resp.body.len()` —— 响应体字节长度
- `callback = ?resp.request.callback` —— callback 标签（Debug 格式）

```rust
#[tracing::instrument(level = "trace", skip(ctx, resp), fields(status = resp.status, body_len = resp.body.len(), callback = ?resp.request.callback))]
```

### Step 2：engine.rs info/warn 日志补充 title 字段（L248-L272）

- 新增 `let title_for_log = resp.title().unwrap_or_default().to_string();`
  - 说明：brief 中的 `unwrap_or_else(|| { None }).unwrap_or_default()` 链式调用类型不通过（`unwrap_or_else` 闭包需返回 `&str` 而非 `Option`），简化为等效的 `unwrap_or_default()`。
  - 必须调用 `.to_string()` 将 `&str` 转为 owned `String`，否则 `resp` 借用未结束无法 move 进 `spider.handle(resp)`。
- `warn!` 消息：`"handle 返回空 (items=0, follows=0): url={}, status={}, title={:?}"`
- `info!` 消息：`"handle 完成: url={}, status={}, title={:?}, items={}, follows={}"`

不调用 `resp.parse()`，避免占用单次解析槽位；HTTP 模式下 `title_for_log` 为空字符串，浏览器模式下有值。

### Step 4：删除示例 novel_crawler.rs 中的诊断 println/eprintln

删除内容：
- 原 L60-L61：`let title = doc.select_one("title")...` + `println!("[首页]...")`
- 原 L98-L105：`if follows.is_empty() { ... } else { ... }` 整个诊断块
- 原 L144：`println!("[详情]...")`
- 原 L184-L185：`println!("[章节]...")`

额外清理：因诊断块被删，`found_selector` 变量成为孤儿代码（声明 + 赋值后从未读取），一并删除以避免编译警告。

保留：所有业务逻辑（选择器循环、follow 构造、NovelItem 组装）以及 main() 末尾的结果展示 println（用户面向的最终输出，非诊断）。

## 测试结果

| 测试 | 命令 | 结果 |
| --- | --- | --- |
| Step 3 焦点测试 | `cargo test --package wisp --lib crawl::engine` | ✅ 27 passed, 0 failed |
| Step 5 示例编译 | `cargo build --example novel_crawler` | ✅ 无错误、无警告 |
| Step 6 lib+bin 全套 | `cargo test --workspace --lib --bins` | ✅ 279 passed, 0 failed |

### 关于 `cargo test --workspace` 的预存在失败

`tests/integration.rs`、`tests/browser_status_code_test.rs`、`tests/cf_bypass_real_test.rs`、`tests/cr_fix_t1_test.rs` 存在编译失败，引用 `Browser`、`TurnstileConfig`、`css_adaptive`、`from_row`、`fetch_browser`、`turnstile_config` 等已被 feature-gate 或重构掉的 API。

**已通过 `git stash` 在 base commit `73ecab8` 上复现这些失败**，确认与本次改动无关。本次改动只触及 `src/crawl/engine.rs` 与 `examples/novel_crawler.rs`，不会影响这些集成测试。

`src/fetcher/mod.rs` 中存在 2 个预存在的 `unused variable: fetcher` 警告，亦非本次改动引入。

## 修改文件

| 文件 | 行范围 | 变更 |
| --- | --- | --- |
| `src/crawl/engine.rs` | L173, L248-L272 | instrument 增加 body_len/callback 字段；info/warn 增加 title_for_log |
| `examples/novel_crawler.rs` | L56-L92, L95-L130, L132-L168 | 删除 4 处诊断 println/eprintln + 孤儿 found_selector |

## 自审发现

- ✅ 7 个步骤全部完成
- ✅ Step 4 列出的 4 处 println/eprintln 全部删除
- ✅ instrument fields 语法正确，编译通过
- ✅ 格式字符串与现有风格一致（`{}` for Display, `{:?}` for Debug）
- ✅ 未触碰任务范围外的代码
- ✅ 未添加 plan 未要求的额外字段
- ✅ lib 测试 279 全绿

## 提交

- SHA: `486db94`
- Subject: `engine: 补充 process_response 日志字段并移除示例手写诊断 println`
- Diff: 2 files changed, 10 insertions(+), 24 deletions(-)
