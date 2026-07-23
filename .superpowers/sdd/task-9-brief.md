### Task 9: 修复 resolve_href 不过滤非 http scheme

**Files:**
- Modify: `src/crawl/mod.rs:166-172`（resolve_href）
- Test: `src/crawl/mod.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: `url::Url::parse` / `Url::join` / `Url::scheme`
- Produces: `SpiderResponse::follow("javascript:...")` 等返回 `None`，不再产生非法请求

**背景：** `resolve_href`（L166-172）对绝对 URL 仅检查 `http://`/`https://` 前缀，但 `url::Url::join` 对 `javascript:`、`mailto:`、`data:` 等 scheme 会构造非 http URL，后续 fetch 时失败或被误处理。

- [ ] **Step 1: 写失败测试**

在 `src/crawl/mod.rs` 的 `#[cfg(test)] mod tests` 末尾追加：

```rust
    #[test]
    fn resolve_href_rejects_non_http_schemes() {
        // 绝对 URL：仅 http/https 通过
        assert!(resolve_href("https://example.com", "https://other.com/p").is_some());
        assert!(resolve_href("https://example.com", "http://other.com/p").is_some());
        // 非 http scheme 应拒绝
        assert!(resolve_href("https://example.com", "javascript:void(0)").is_none(),
            "javascript: scheme 应被拒绝");
        assert!(resolve_href("https://example.com", "mailto:a@b.com").is_none(),
            "mailto: scheme 应被拒绝");
        assert!(resolve_href("https://example.com", "data:text/html,xxx").is_none(),
            "data: scheme 应被拒绝");
        // 相对链接仍正常解析
        assert!(resolve_href("https://example.com/a/", "b").is_some());
        assert_eq!(resolve_href("https://example.com/a/", "b"), Some("https://example.com/a/b".into()));
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib crawl::tests::resolve_href_rejects_non_http_schemes 2>&1 | tail -15`
Expected: FAIL — `javascript:` 等经 `Url::join` 后返回 Some（非 None）。

- [ ] **Step 3: 修复 resolve_href**

修改 `src/crawl/mod.rs` L166-172：

```rust
fn resolve_href(base: &str, href: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    let base_url = url::Url::parse(base).ok()?;
    let joined = base_url.join(href).ok()?;
    // 仅接受 http/https 结果（过滤 javascript: mailto: data: 等被 join 构造的非法 URL）
    if joined.scheme() == "http" || joined.scheme() == "https" {
        Some(joined.to_string())
    } else {
        None
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib crawl::tests::resolve_href 2>&1 | tail -10`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add src/crawl/mod.rs
git commit -m "fix(crawl): resolve_href 过滤非 http/https scheme

- 对 Url::join 结果检查 scheme，拒绝 javascript:/mailto:/data: 等
- 修复 follow 非法链接产生无效请求的问题"
```

---

