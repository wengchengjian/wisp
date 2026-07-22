# Task 5 报告：Spider trait 加 patterns/matches/until 方法

## 状态
DONE

## 改动文件
- `f:\project\wisp\src\crawl\mod.rs`（+22 行）

## 改动内容
在 `Spider` trait 的 `fn schedule()` 之后追加 3 个带默认实现的方法：

1. `fn patterns(&self) -> Vec<String>` — 返回空 Vec（匹配所有 URL）
2. `fn matches(&self, url: &str) -> bool` — 默认遍历 patterns()，任一正则匹配即 true；空 patterns 匹配所有
3. `fn until(&self) -> Arc<dyn StopCondition>` — 默认返回 `Arc::new(NeverStop)`

## 依赖确认（改前已就绪）
- `use std::sync::Arc;` 已在 mod.rs 第 30 行
- `StopCondition`、`NeverStop` 通过第 25 行 `pub use stop::{...}` 导入
- `regex = "1"` 已在 Cargo.toml

## 验证
- `cargo build --lib`：PASS（exit 0，仅有项目原有的 8 条 warning，与本次改动无关）
- `cargo test --lib`：PASS（159 passed; 0 failed; 0 ignored）

## 提交
- commit `8e9e7a9`：`feat: Spider trait 加 patterns/matches/until 钩子`
- 仅修改 `src/crawl/mod.rs` 1 个文件

## 自审
- ✅ 新方法全部带默认实现，现有 Spider 实现（DummySpider、CountSpider、OneSpider、ClosureSpider 等）无需改动
- ✅ trait 方法签名与 task brief 完全一致
- ✅ `matches` 默认实现中正则编译失败时回退为 false（`unwrap_or(false)`），避免 panic
- ✅ `until` 返回 `Arc<dyn StopCondition>`，与 stop 模块定义的类型吻合
- ✅ 无新增 import，无 Cargo.toml 改动，最小化变更

## 关切
无。所有现有测试通过，新方法不影响向后兼容性。
