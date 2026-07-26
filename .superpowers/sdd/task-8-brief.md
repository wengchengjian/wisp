# Task 8 (Round 2): Clippy 自动修复 + Code Review

**Files:**
- Modify: 全项目（自动修复 + 手动补充）

**Interfaces:**
- Consumes: `cargo clippy --fix`、`cargo fmt`
- Produces: 0 clippy 警告

## 当前 clippy 基线

- lib test: 601 warnings (523 duplicates, 56 auto-fixable suggestions)
- 集成测试: ~15 warnings
- benches: ~3 warnings
- examples: 1 warning

主要类别：
- `uninlined_format_args`（最多，`format!("{}", x)` → `format!("{x}")`）
- `unnecessary_literal_bound`（trait 方法返回 `&str` 但实际是字面量 → `&'static str`）
- `type_complexity`（`Mutex<HashMap<(String, String), (Vec<u8>, Option<Instant>)>>` → 抽 type alias）
- `duration_suboptimal_units`（`Duration::from_secs(3600)` → `Duration::from_hours(1)`）
- `items_after_statements`（测试中 struct/impl 定义在语句之后）
- `expect_fun_call`（`.expect(&format!(...))` → `.unwrap_or_else(|_| panic!(...))`）
- `unnecessary_sort_by`（`sort_by(|a,b| b.cmp(a))` → `sort_by_key(|b| Reverse(b))`）
- `new_without_default`（`TimingLayer::new` 缺 `Default` impl）
- `doc_lazy_continuation`（doc 注释续行未缩进）
- `unnecessary_get_then_check`（`.get(&x).is_some()` → `.contains_key(&x)`）
- `unused_imports`、`unused_mut`

## Steps

### Step 1: 自动修复

```bash
cd /home/weng/wisp
cargo clippy --fix --all-targets --all-features --allow-dirty --allow-no-vcs
cargo fmt --all
```

Expected: 自动修复 `uninlined_format_args`、`duration_suboptimal_units`、`unnecessary_get_then_check`、`unused_mut`、`unused_imports`、`unnecessary_sort_by`、`doc_lazy_continuation` 等。

### Step 2: 编译验证

Run: `cd /home/weng/wisp && cargo build --all-features`
Expected: 编译通过

如果自动修复引入编译错误（如 `format!("{url}")` 但 `url` 是 `&Url` 类型无 `Display` 内联支持），手动修复：

```rust
// 自动修复误改
format!("{url}")  // 编译失败

// 手动修复
format!("{}", url)  // 恢复
// 或
format!("{}", url.as_str())  // 显式转 &str
```

### Step 3: 全量回归

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

### Step 4: 检查剩余 clippy 警告

Run: `cd /home/weng/wisp && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tee /tmp/clippy_remaining.log | tail -100`
Expected: 列出剩余无法自动修复的警告

### Step 5: 手动修复 type_complexity

搜索 `type_complexity` 警告，抽取 type alias：

```rust
// 旧（src/storage/mod.rs:224）
struct MockStore {
    data: Mutex<HashMap<(String, String), (Vec<u8>, Option<Instant>)>>,
}

// 新
type MockStoreData = HashMap<(String, String), (Vec<u8>, Option<Instant>)>;
struct MockStore {
    data: Mutex<MockStoreData>,
}
```

### Step 6: 处理 unnecessary_literal_bound（不要改 trait 签名）

**已确认决策：不修改 Spider trait 签名**。

原因：`src/crawl/builder.rs:239` 的 `ClosureSpider` 返回 `&self.name`（动态 String），改 trait 为 `&'static str` 会破坏该实现。`ClosureSpider` 是生产代码核心路径，不能为消除警告而破坏。

`unnecessary_literal_bound` 警告出现在测试中的 Spider 实现（如 `src/crawl/mod.rs:258,366,405`、`src/crawl/engine.rs:863`、`src/mcp/tools.rs:108`），它们返回字面量 `"dummy"`/`"count"`/`"one"`/`"mcp_simple"`。

**修复方式**：在每个被警告的 impl 上加 `#[allow(clippy::unnecessary_literal_bound)]`：

```rust
#[allow(clippy::unnecessary_literal_bound)]
impl Spider for FooSpider {
    fn name(&self) -> &str { "foo" }
}
```

或者，如果测试中的 struct 已经被 `items_after_statements` 标记，可以同时把 struct/impl 移出函数外（一并举解决两个警告）。

**绝对不要**：
- 修改 `Spider` trait 的 `name` 签名为 `&'static str`
- 修改 `ClosureSpider` 的 `name` 字段类型

### Step 7: 手动修复 items_after_statements（测试中）

测试函数中的 struct/impl 定义在语句之后会触发此警告。修复方式：

```rust
// 旧
#[tokio::test]
async fn test_foo() {
    let url = "http://example.com";
    struct FooSpider { ... }
    impl Spider for FooSpider { ... }
    // ...
}

// 新（struct/impl 移到函数顶部或函数外）
struct FooSpider { ... }
impl Spider for FooSpider { ... }

#[tokio::test]
async fn test_foo() {
    let url = "http://example.com";
    // ...
}
```

或者用 `#[allow(clippy::items_after_statements)]` 标注测试函数（如果移动会破坏测试可读性）。

### Step 8: 手动修复 expect_fun_call

```rust
// 旧
.expect(&format!("invalid json: {}", stdout))

// 新
.unwrap_or_else(|_| panic!("invalid json: {}", stdout))
```

### Step 9: 手动修复 new_without_default

```rust
// 旧
impl TimingLayer {
    pub fn new() -> Self { ... }
}

// 新
impl Default for TimingLayer {
    fn default() -> Self { Self::new() }
}

impl TimingLayer {
    pub fn new() -> Self { ... }
}
```

### Step 10: 验证 0 警告

Run: `cd /home/weng/wisp && cargo clippy --all-targets --all-features -- -D warnings`
Expected: 命令成功退出（exit code 0），0 警告

### Step 11: 集成验证（banzhu-rs）

```bash
cd /home/weng/banzhu-rs && cargo build
```

Expected: 编译通过（如果 Spider trait 签名改了 `&'static str`，banzhu-rs 的实现也需要同步改）

如果 banzhu-rs 编译失败，修复 banzhu-rs 中对应的 trait 实现（`fn name(&self) -> &str` → `fn name(&self) -> &'static str`）。

### Step 12: 最终全量回归

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

### Step 13: 提交

```bash
cd /home/weng/wisp
git add -A
git commit -m "chore(clippy): 修复全部 clippy 警告 + 自动格式化"
```

如果 banzhu-rs 也改了，单独提交：
```bash
cd /home/weng/banzhu-rs
git add -A
git commit -m "chore: 同步 clippy 自动修复"
```

**注意**：由于 Step 6 已确认不修改 Spider trait 签名，banzhu-rs 应该不需要因 trait 签名变更而修改。但如果 clippy --fix 在 wisp 中改了公共 API（如 pub fn 签名），需要同步修复 banzhu-rs。

## 验证

- `cargo clippy --all-targets --all-features -- -D warnings` 退出码 0
- `cargo test --all-features` 全部 PASS
- `cd /home/weng/banzhu-rs && cargo build` 编译通过
- 提交信息符合规范

## 注意事项

1. **不要为了一味消除警告而破坏功能** — 如果某个警告修复会改变语义，用 `#[allow(clippy::xxx)]` 标注并加注释说明原因
2. **`unnecessary_literal_bound` 涉及 trait 签名变更** — 这会影响 banzhu-rs，需要同步修复
3. **`items_after_statements` 在测试中** — 如果移动 struct 会破坏测试结构，用 `#[allow]` 标注
4. **保留中文注释** — `cargo fmt` 不会动注释，但自动修复可能调整代码位置
5. **不要删除 `#![warn(missing_docs)]`** — 保持文档警告开启
6. 如果某个警告是 clippy 误判，用 `#[expect(clippy::xxx)]`（Rust 1.81+）而非 `#[allow]`
