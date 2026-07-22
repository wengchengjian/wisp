# Task 6: SpiderBuilder/ClosureSpider 加 patterns 与 until 支持

**Files:**
- Modify: `src/crawl/builder.rs`

## Steps

### Step 1: SpiderBuilder 加字段

在 `SpiderBuilder` 结构体定义中（约第 41-56 行），在 `is_blocked_fn` 字段之后加两个字段：

```rust
    patterns: Vec<String>,
    until_cond: Arc<dyn super::stop::StopCondition>,
```

完整结构体应为：
```rust
pub struct SpiderBuilder {
    name: String,
    start_urls: Vec<String>,
    allowed_domains: HashSet<String>,
    concurrent: u32,
    delay: Duration,
    obey_robots: bool,
    max_retries: u32,
    fetcher_config: http::Config,
    fetch_mode: crate::fetcher::FetchMode,
    auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
    auto_exclude: HashSet<String>,
    parse_fn: Option<ParseFn>,
    async_parse_fn: Option<AsyncParseFn>,
    is_blocked_fn: Option<Box<dyn Fn(&SpiderResponse) -> bool + Send + Sync + 'static>>,
    patterns: Vec<String>,
    until_cond: Arc<dyn super::stop::StopCondition>,
}
```

### Step 2: SpiderBuilder::new 初始化

在 `new()` 函数中（约第 60-77 行），在 `is_blocked_fn: None,` 之后加：

```rust
            patterns: Vec::new(),
            until_cond: Arc::new(super::NeverStop),
```

### Step 3: 加 .patterns() 和 .until() 方法

在 `SpiderBuilder` 的 `is_blocked` 方法之后、`build` 方法之前，加两个 builder 方法：

```rust
    /// 设置 URL 匹配模式（正则字符串数组）。任一匹配即处理该 URL。
    pub fn patterns(mut self, patterns: Vec<String>) -> Self {
        self.patterns = patterns;
        self
    }

    /// 设置终止条件策略。
    pub fn until<C: super::stop::StopCondition + 'static>(mut self, cond: C) -> Self {
        self.until_cond = Arc::new(cond);
        self
    }
```

### Step 4: ClosureSpider 加字段

在 `ClosureSpider` 结构体定义中（约第 206-221 行），在 `is_blocked_fn` 字段之后加：

```rust
    patterns: Vec<String>,
    until_cond: Arc<dyn super::stop::StopCondition>,
```

### Step 5: build() 传递新字段

在 `build()` 方法中（约第 186-202 行），在 `is_blocked_fn: self.is_blocked_fn,` 之后加：

```rust
            patterns: self.patterns,
            until_cond: self.until_cond,
```

### Step 6: ClosureSpider impl Spider

在 `ClosureSpider` 的 `impl Spider for ClosureSpider` 块中，在 `is_blocked` 方法之后加：

```rust
    fn patterns(&self) -> Vec<String> { self.patterns.clone() }

    fn until(&self) -> Arc<dyn super::stop::StopCondition> {
        Arc::clone(&self.until_cond)
    }
```

### Step 7: 编译验证

Run: `cargo build --lib`
Expected: PASS

### Step 8: 运行测试

Run: `cargo test --lib`
Expected: PASS（现有 builder 测试不受影响）

### Step 9: Commit

```bash
git add src/crawl/builder.rs
git commit -m "feat: SpiderBuilder/ClosureSpider 支持 patterns 与 until"
```

## 注意

- `Arc` 需要导入：确认 builder.rs 顶部有 `use std::sync::Arc;`，如果没有则添加
- `super::stop::StopCondition` 路径：通过 `super::` 访问父模块的 stop 模块（因为 stop 是 `pub mod stop;`）
- `super::NeverStop` 同理
