# Task 5: Spider trait 加 patterns/matches/until 方法

**Files:**
- Modify: `src/crawl/mod.rs`（Spider trait 定义，约第 155-190 行）

## Steps

### Step 1: 修改 Spider trait

在 `src/crawl/mod.rs` 的 `Spider` trait 定义中，在 `fn schedule(&self) -> Option<&str> { None }` 之后（trait 的最后），加 3 个新方法：

```rust
    // === 路由与终止（新增） ===

    /// URL 匹配模式（字符串数组，内部自动编译为正则）。默认空 Vec（匹配所有）。
    fn patterns(&self) -> Vec<String> { Vec::new() }

    /// URL 匹配判定。默认实现遍历 patterns()，任一正则匹配即返回 true。
    /// patterns() 为空时匹配所有 URL。
    fn matches(&self, url: &str) -> bool {
        let patterns = self.patterns();
        if patterns.is_empty() {
            return true;
        }
        patterns.iter().any(|p| {
            regex::Regex::new(p).map(|re| re.is_match(url)).unwrap_or(false)
        })
    }

    /// 终止条件。默认永不停止（由引擎 max_pages 兜底）。
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(NeverStop)
    }
```

**注意**：
- `Arc` 和 `StopCondition` / `NeverStop` 已在前面的 task 中导入（`pub use stop::{...}` 在 mod.rs 顶部）
- 如果 `Arc` 未在 mod.rs 作用域中，需要确认 `use std::sync::Arc;` 已存在（检查文件顶部 imports）
- `regex` crate 已在 Cargo.toml 中（`regex = "1"`）

### Step 2: 确认 imports

检查 `src/crawl/mod.rs` 顶部是否有：
- `use std::sync::Arc;`（应该已有）
- `StopCondition` 和 `NeverStop` 已通过 `pub use stop::{...}` 导入

如果 `Arc` 未导入，添加 `use std::sync::Arc;`。

### Step 3: 编译验证

Run: `cargo build --lib`
Expected: PASS（现有 Spider 实现不需要改动，因为新方法都有默认实现）

### Step 4: 运行测试

Run: `cargo test --lib`
Expected: PASS（现有测试不受影响）

### Step 5: Commit

```bash
git add src/crawl/mod.rs
git commit -m "feat: Spider trait 加 patterns/matches/until 钩子"
```
