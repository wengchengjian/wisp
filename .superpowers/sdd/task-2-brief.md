## Task 2: Response 添加 meta_str/meta_u64 类型化访问器

**Files:**
- Modify: `src/fetcher/response.rs` (impl Response 块, 在 L456 `}` 前插入)
- Test: `src/fetcher/response.rs` tests 模块（L460+）

**Interfaces:**
- Consumes: `Request::meta: Value`（[response.rs#L160](file:///home/weng/wisp/src/fetcher/response.rs#L160)）
- Produces:
  - `pub fn meta_str(&self, key: &str) -> &str`
  - `pub fn meta_u64(&self, key: &str) -> u64`

- [ ] **Step 1: 写失败测试**

在 [src/fetcher/response.rs](file:///home/weng/wisp/src/fetcher/response.rs) 的 `#[cfg(test)] mod tests` 块末尾追加：

```rust
#[test]
fn test_response_meta_str_returns_value_when_present() {
    let mut req = Request::get("https://example.com/");
    req.meta = serde_json::json!({"title": "你好", "author": ""});
    let resp = Response::from_http(
        200,
        "https://example.com/".into(),
        std::collections::HashMap::new(),
        b"<html></html>".to_vec(),
        "text/html".into(),
        req,
    );
    assert_eq!(resp.meta_str("title"), "你好");
    assert_eq!(resp.meta_str("author"), "");
}

#[test]
fn test_response_meta_str_returns_empty_when_missing() {
    let req = Request::get("https://example.com/");
    let resp = Response::from_http(
        200,
        "https://example.com/".into(),
        std::collections::HashMap::new(),
        b"<html></html>".to_vec(),
        "text/html".into(),
        req,
    );
    // meta 为 Null
    assert_eq!(resp.meta_str("title"), "");
}

#[test]
fn test_response_meta_str_returns_empty_when_meta_not_object() {
    let mut req = Request::get("https://example.com/");
    req.meta = serde_json::Value::Null;
    let resp = Response::from_http(
        200,
        "https://example.com/".into(),
        std::collections::HashMap::new(),
        b"<html></html>".to_vec(),
        "text/html".into(),
        req,
    );
    assert_eq!(resp.meta_str("anything"), "");
}

#[test]
fn test_response_meta_u64_returns_value_when_present() {
    let mut req = Request::get("https://example.com/");
    req.meta = serde_json::json!({"chapter_index": 42});
    let resp = Response::from_http(
        200,
        "https://example.com/".into(),
        std::collections::HashMap::new(),
        b"<html></html>".to_vec(),
        "text/html".into(),
        req,
    );
    assert_eq!(resp.meta_u64("chapter_index"), 42);
}

#[test]
fn test_response_meta_u64_returns_zero_when_missing_or_invalid() {
    let mut req = Request::get("https://example.com/");
    req.meta = serde_json::json!({"chapter_index": "not a number"});
    let resp = Response::from_http(
        200,
        "https://example.com/".into(),
        std::collections::HashMap::new(),
        b"<html></html>".to_vec(),
        "text/html".into(),
        req,
    );
    assert_eq!(resp.meta_u64("chapter_index"), 0);
    assert_eq!(resp.meta_u64("missing"), 0);
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --package wisp --lib fetcher::response::tests::test_response_meta`
Expected: FAIL，编译错误 `no method named meta_str/meta_u64 found`

- [ ] **Step 3: 实现 meta_str 和 meta_u64**

在 [src/fetcher/response.rs#L456](file:///home/weng/wisp/src/fetcher/response.rs#L456) `follow_meta` 方法之后、`impl Response` 块的 `}` 之前插入：

```rust
/// 读取 meta 中的字符串字段。缺失/类型不符/.meta 非 Object 时返回空字符串。
///
/// 替代样板代码：`meta.get("x").and_then(|v| v.as_str()).unwrap_or("").to_string()`。
pub fn meta_str(&self, key: &str) -> &str {
    self.request
        .meta
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

/// 读取 meta 中的 u64 字段。缺失/类型不符时返回 0。
pub fn meta_u64(&self, key: &str) -> u64 {
    self.request
        .meta
        .get(key)
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}
```

注意：`meta_str` 返回 `&str`（借自 `Value`），避免一次 `to_string()` 分配。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --package wisp --lib fetcher::response::tests::test_response_meta`
Expected: PASS（5 个测试全绿）

- [ ] **Step 5: 重构示例使用 meta_str**

修改 [examples/novel_crawler.rs](file:///home/weng/wisp/examples/novel_crawler.rs) 的 detail 和 chapter handler：

detail handler（原 [L113-L117](file:///home/weng/wisp/examples/novel_crawler.rs#L113-L117)）：
```rust
let title = resp.meta_str("title").to_string();
```

chapter handler（原 [L151-L155](file:///home/weng/wisp/examples/novel_crawler.rs#L151-L155)）：
```rust
let title = resp.meta_str("title").to_string();
let author = resp.meta_str("author").to_string();
let chapter_title = resp.meta_str("chapter_title").to_string();
let chapter_index = resp.meta_u64("chapter_index") as usize;
```

- [ ] **Step 6: 编译并运行完整测试**

Run: `cargo build --example novel_crawler && cargo test --workspace`
Expected: PASS

- [ ] **Step 7: 提交**

```bash
git add src/fetcher/response.rs examples/novel_crawler.rs
git commit -m "response: 新增 meta_str/meta_u64 类型化访问器替代样板代码"
```

---

