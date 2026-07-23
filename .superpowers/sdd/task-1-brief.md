### Task 1: P1-5 Method::as_str() DRY 转换

**Files:**
- Modify: `src/crawl/mod.rs:51-53`
- Modify: `src/crawl/engine.rs:309-314`
- Modify: `src/crawl/middleware/builtin.rs:283-288, 307-312`
- Test: `src/crawl/mod.rs`（追加到现有 `tests` 模块）

**Interfaces:**
- Produces: `pub fn Method::as_str(&self) -> &'static str`，返回 `"GET"/"POST"/"PUT"/"DELETE"`。

- [ ] **Step 1: 写失败测试 — Method::as_str 返回标准 HTTP 动词**

在 `src/crawl/mod.rs` 现有 `#[cfg(test)] mod tests` 模块末尾追加（该模块已有其他测试，找到其 `}` 前插入）：

```rust
    fn test_method_as_str_returns_standard_verbs() {
        assert_eq!(Method::Get.as_str(), "GET");
        assert_eq!(Method::Post.as_str(), "POST");
        assert_eq!(Method::Put.as_str(), "PUT");
        assert_eq!(Method::Delete.as_str(), "DELETE");
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib crawl::tests::test_method_as_str_returns_standard_verbs`
Expected: 编译失败 — `as_str` 方法不存在（`no method named as_str`）。

- [ ] **Step 3: 实现 Method::as_str**

在 `src/crawl/mod.rs:53` 的 `pub enum Method { Get, Post, Put, Delete }` 下方新增 impl 块：

```rust
impl Method {
    /// 返回标准 HTTP 动词字符串（大写）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}
```

- [ ] **Step 4: 替换 engine.rs 的 method_str match**

`src/crawl/engine.rs:308-314` 当前：

```rust
    // 提前计算 method_str（缓存查询与写入都需要）
    let method_str = match req.method {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
    };
```

替换为：

```rust
    // 提前计算 method_str（缓存查询与写入都需要）
    let method_str = req.method.as_str();
```

- [ ] **Step 5: 替换 builtin.rs 两处 method_str match**

`src/crawl/middleware/builtin.rs:283-288`：

```rust
        let method_str = match req.method {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        };
```

替换为：

```rust
        let method_str = req.method.as_str();
```

`src/crawl/middleware/builtin.rs:307-312`：

```rust
            let method_str = match resp.request.method {
                Method::Get => "GET",
                Method::Post => "POST",
                Method::Put => "PUT",
                Method::Delete => "DELETE",
            };
```

替换为：

```rust
            let method_str = resp.request.method.as_str();
```

- [ ] **Step 6: 运行测试验证通过**

Run: `cargo test --lib crawl::tests::test_method_as_str_returns_standard_verbs && cargo build`
Expected: 测试 PASS；编译无错（engine.rs/builtin.rs 中 `Method` 仍被引用，无未使用 import 警告）。

- [ ] **Step 7: 提交**

```bash
git add src/crawl/mod.rs src/crawl/engine.rs src/crawl/middleware/builtin.rs
git commit -m "refactor: Method::as_str() DRY 3 处字符串转换 (P1-5)"
```

---

