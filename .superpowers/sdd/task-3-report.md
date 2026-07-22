# Task 3 报告：删除 parse_fn/async_parse_fn 冗余，统一到 on() API

## Status: DONE

## Commits
- `d0de503` refactor(builder): 删除 parse_fn/async_parse_fn，统一到 on() API

## 变更摘要

### src/crawl/builder.rs
- 删除 `ParseFn` / `AsyncParseFn` 类型别名
- 删除 `SpiderBuilder` 的 `parse_fn` / `async_parse_fn` 字段
- 删除 `SpiderBuilder::parse()` / `parse_async()` builder 方法
- 删除 `ClosureSpider` 的 `parse_fn` / `async_parse_fn` 字段
- `ClosureSpider::parse()` 改为兜底空实现 `(vec![], vec![])`
- `build()` 断言改为 `!self.handlers.is_empty()`，panic 消息更新为「必须至少注册一个 handler（通过 on()）」
- `handle()` 路由回退逻辑：无 default handler 时直接返回空（不再回退 parse 闭包）
- 文档注释示例从 `.parse(|resp| {...})` 迁移到 `.on("default", |resp| async move {...})`
- 内部测试迁移：
  - `test_spider_builder_basic` / `test_spider_builder_allowed_domains` / `test_closure_spider_custom_is_blocked` → `.on("default", ...)`
  - `test_closure_spider_parse` → `test_closure_spider_default_handler`（改用 `handle()`）
  - `test_closure_spider_parse_async` → `test_closure_spider_async_handler`（改用 `handle()`）
  - `test_spider_builder_no_parse_panics` → `test_spider_builder_no_handler_panics`（panic 消息更新）
  - `test_closure_spider_handle_fallback_to_parse` → `test_closure_spider_handle_default_handler`（语义改为 default handler 路由）

### tests/（全部 .parse() 调用迁移）
- `tests/builder_api_test.rs`：4 处 `.parse()` → `.on("default", ...)`；`test_spider_builder_parse_with_follow` 改用 `spider.handle(resp)`
- `tests/callback_routing_test.rs`：`test_callback_no_handler_falls_back_to_parse` → `test_callback_default_handler_serves_no_callback`；更新注释
- `tests/auto_mode_test.rs`：2 处 `.parse()` → `.on("default", ...)`
- `tests/cf_bypass_real_test.rs`：1 处 `.parse()` → `.on("default", ...)`（GBK 文件，ASCII 定向编辑，未触及 GBK 注释）
- `tests/real_scrape_test.rs`：2 处 `.parse()` → `.on("default", ...)`（GBK 文件，同上）

## 测试摘要

| 命令 | 结果 |
|------|------|
| `cargo build --lib` | ✅ 通过（仅预先存在的 7 个 warning，无 error） |
| `cargo test --lib crawl::builder` | ✅ 8 passed; 0 failed |
| `cargo test --test builder_api_test` | ✅ 10 passed; 0 failed |
| `cargo test --test callback_routing_test` | ✅ 9 passed; 0 failed |
| `cargo test --test auto_mode_test --no-run` | ✅ 编译通过（仅 1 个预先存在的 unused import warning） |

## Concerns

1. **文档文件未更新**：`CLAUDE.md`（第 79 行）和 `README.md`（第 90 行）仍含旧 `.parse(|resp| {...})` 示例。brief 的 grep 迁移范围限定为 `src/` 和 `tests/`，故未触及文档文件。这些示例现已过时（`.parse()` builder 方法已删除），建议后续单独更新。

2. **GBK 编码测试文件**：`tests/real_scrape_test.rs`、`tests/cf_bypass_real_test.rs` 按 CLAUDE.md 记录存在预先的 GBK 编码问题（非 UTF-8），原本即无法编译。本次对其中的 `.parse()` 调用做了 ASCII 定向替换（未触及 GBK 注释字节），但这些文件仍因 GBK 编码问题无法编译——此为预先存在问题，非本次引入。

3. **`builder_api_test.rs` 预先存在的 unused import warning**（`wisp::http`、`wisp::FetchMode`）：本次迁移未引入新 warning，这些 import 在迁移前即未被使用。
