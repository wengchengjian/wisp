# Task 6 报告: SpiderBuilder/ClosureSpider 加 patterns 与 until 支持

## 状态
DONE

## 提交
- `371ee65` feat: SpiderBuilder/ClosureSpider 支持 patterns 与 until

## 测试
- `cargo build --lib`: PASS (exit 0)
- `cargo test --lib`: 159 passed; 0 failed; 0 ignored

## 改动文件
- `src/crawl/builder.rs` (+27 行)

## 实现细节

按 brief 的 7 个步骤逐一完成，所有改动集中在 `src/crawl/builder.rs`：

1. **导入 Arc** (line 25): 添加 `use std::sync::Arc;`
2. **SpiderBuilder 加字段** (lines 57-58): `patterns: Vec<String>` 和 `until_cond: Arc<dyn super::stop::StopCondition>`，位于 `is_blocked_fn` 之后
3. **new() 初始化** (lines 79-80): `patterns: Vec::new()`, `until_cond: Arc::new(super::NeverStop)`
4. **builder 方法** (lines 182-192):
   - `.patterns(Vec<String>) -> Self`
   - `.until<C: super::stop::StopCondition + 'static>(cond: C) -> Self`
5. **ClosureSpider 加字段** (lines 240-241): 同 SpiderBuilder
6. **build() 传递** (lines 218-219): `patterns: self.patterns`, `until_cond: self.until_cond`（move 语义，无需 clone）
7. **impl Spider** (lines 276-280):
   - `fn patterns(&self) -> Vec<String> { self.patterns.clone() }`
   - `fn until(&self) -> Arc<dyn super::stop::StopCondition> { Arc::clone(&self.until_cond) }`

## 自审

- 路径正确：`super::stop::StopCondition` 经由 `pub mod stop;` 可达；`super::NeverStop` 经由 `mod.rs` 的 `pub use stop::{... NeverStop ...}` 重导出可达
- `until()` 返回类型与 trait 定义一致（trait 在 `mod.rs:215` 声明 `Arc<dyn StopCondition>`，ClosureSpider 实现返回 `Arc<dyn super::stop::StopCondition>`，同一类型）
- `Arc::clone(&self.until_cond)` 是克隆 Arc 的惯用写法（语义明确，避免 `.clone()` 歧义）
- `build()` 中 `until_cond: self.until_cond` 直接 move Arc（无需 clone），零成本
- 现有 5 个 builder 测试全部通过，无回归

## 顾虑
无
