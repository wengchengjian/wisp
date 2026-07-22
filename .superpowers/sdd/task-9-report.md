# Task 9 报告：多 Spider 路由 + until 终止 E2E 骨架测试

## 实现内容

创建了 `tests/multi_spider_test.rs`，包含：

1. **ListSpider 骨架**：从 `list/1` 起爬，patterns 匹配 `test\.example/list/\d+`，`until()` 返回 `MaxPages(50)`，`parse` 产出 list/N+1 与 detail/X 两条 follow 请求。
2. **DetailSpider 骨架**：空 start_urls，patterns 匹配 `test\.example/detail/\d+`，`until()` 返回 `NeverStop`（受限于上游 ListSpider）。
3. **`test_max_pages_condition` 单元测试**：构造 `StopContext { pages: 50, ... }`，断言 `MaxPages(50).should_stop(&ctx) == true`。

骨架不跑真实 HTTP，仅验证 `Spider` trait 钩子（patterns/until/parse）与 `SpiderResponse::follow` 调用链编译通过，并校验 `StopCondition` 逻辑。

## 测试与结果

```
cargo test --test multi_spider_test
```

```
running 1 test
test test_max_pages_condition ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

编译通过，测试通过。退出码 0。

剩余 2 个 `dead_code` 警告（`ListSpider` / `DetailSpider` 未被构造）属预期：这是骨架代码，将在 Task 10 HTTP 路由 E2E 测试中实例化。库代码的 7 个警告为既有遗留，与本次改动无关。

## 适配 plan 代码的改动

plan 代码转录后首次编译有 1 错 1 警，做了以下适配（保持测试意图不变）：

1. **错误修复（E0599 `should_stop` 方法未找到）**：plan 在测试函数内部 `use wisp::crawl::StopContext;` 但未引入 `StopCondition` trait。`should_stop` 是 trait 方法，trait 必须在作用域内才能调用。
   - 修复：将 `StopCondition` 提升到文件顶部 import，并连同 `StopContext` 一起从函数内移到顶部 `use wisp::crawl::{MaxPages, NeverStop, StopContext};`。

2. **警告清理（unused import `Engine`）**：plan 的 `use wisp::crawl::{Spider, SpiderRequest, SpiderResponse, Engine};` 中 `Engine` 未被使用（Task 10 才会用到）。
   - 修复：用 `StopCondition` 替换 `Engine`（同一 import 行），消除 unused 警告。

3. **微调（`resp` → `_resp`）**：`DetailSpider::parse` 的 `resp` 参数未被使用，加下划线前缀避免 unused 警告。

这些适配均为编译/警告层面，未改变测试逻辑与骨架结构。

## API 一致性核对

brief 提示的核对项全部通过：

| 项 | 状态 |
|---|---|
| `SpiderResponse::follow(&self, href: &str) -> Option<SpiderRequest>` | 存在，签名匹配 (`src/crawl/mod.rs:120`) |
| `wisp::crawl::{MaxPages, NeverStop, StopContext}` 重导出 | 已在 `src/crawl/mod.rs:25` 重导出 |
| `StopContext` 字段 pages/items/errors/in_flight/elapsed/queue_size | 完全匹配 (`src/crawl/stop.rs:8-21`) |
| `Spider::patterns() -> Vec<String>` / `until() -> Arc<dyn StopCondition>` | 签名匹配 (`src/crawl/mod.rs:201,216`) |
| `Spider::parse(&self, SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>)` | 签名匹配 (`src/crawl/mod.rs:166`) |

## 变更文件

- 新增：`tests/multi_spider_test.rs`（62 行）

## Commit

- `9b7d117` — test: 多 Spider 路由与 until 终止策略骨架测试
