# Task 1: 新增依赖与 ParseError 变体

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/error.rs`

## Step 1: 在 Cargo.toml 的 [dependencies] 追加 sxd 依赖

在 `chrono = ...` 这一行之后追加：

```toml
# XPath 1.0 完整查询（阶段 2：sxd-xpath 懒解析）
sxd-document = "0.3"
sxd-xpath = "0.4"
```

## Step 2: 在 src/error.rs 的 WispError enum 追加 ParseError 变体

在 `McpError(String)` 变体之后追加：

```rust
    #[error("Parse error: {0}")]
    ParseError(String),
```

## Step 3: 运行 cargo check 验证编译

Run: `cargo check`
Expected: 编译通过（sxd 依赖会被拉取）

## Step 4: 提交

```bash
git add Cargo.toml src/error.rs
git commit -m "feat: 新增 sxd-document/sxd-xpath 依赖与 ParseError 变体"
```
