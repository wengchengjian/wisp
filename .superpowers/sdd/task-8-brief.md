### Task 8: 修复 RequestCache 键忽略 HTTP 方法

**Files:**
- Modify: `src/crawl/runtime/request_cache.rs:40-52`（get/put/invalidate 签名加 method）
- Modify: `src/crawl/engine.rs:142-157, 241-250`（调用处传 method）
- Test: `src/crawl/runtime/request_cache.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: `Method`（crawl/mod.rs:53），`RequestCache.inner: moka::Cache<String, CachedEntry>`
- Produces: `RequestCache::{get,put,invalidate}` 新增 `method: &str` 参数；键为 `"{method} {url}"`；POST/GET 同 URL 不冲突

**背景：** `RequestCache`（request_cache.rs:40-47）键只用 URL。`process_request`（engine.rs:142-157）查询时也只用 `req.url`。导致 POST 与 GET 同 URL 共享缓存，返回错误响应。dev_mode 的 SQLite 缓存用 `(url, method)` 正确，两者不一致。

- [ ] **Step 1: 写失败测试 — POST 与 GET 同 URL 不共享缓存**

在 `src/crawl/runtime/request_cache.rs` 的 `#[cfg(test)]` 末尾追加：

```rust
    #[tokio::test]
    async fn cache_key_includes_method() {
        let cache = RequestCache::new(100, Duration::from_secs(60));
        let get_entry = CachedEntry {
            status: 200,
            headers: HashMap::new(),
            body: b"GET-RESPONSE".to_vec(),
        };
        // 存 GET 响应
        cache.put("GET", "https://example.com/api", get_entry).await;

        // GET 命中
        let got = cache.get("GET", "https://example.com/api").await;
        assert!(got.is_some(), "GET 应命中");

        // POST 不应命中 GET 的缓存
        let post = cache.get("POST", "https://example.com/api").await;
        assert!(post.is_none(), "POST 不应命中 GET 缓存，实际 {:?}", post);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib crawl::runtime::request_cache::tests::cache_key_includes_method 2>&1 | tail -15`
Expected: 编译失败（put/get 签名不匹配）或 FAIL（同 URL 命中）。

- [ ] **Step 3: 修改 RequestCache 签名加 method**

修改 `src/crawl/runtime/request_cache.rs` L26-58：

```rust
impl RequestCache {
    pub fn new(max_entries: u64, ttl: Duration) -> Self {
        Self {
            inner: Cache::builder()
                .max_capacity(max_entries)
                .time_to_live(ttl)
                .build(),
        }
    }

    /// 构造缓存键："{method} {url}"，区分不同 HTTP 方法的响应。
    fn cache_key(method: &str, url: &str) -> String {
        format!("{} {}", method, url)
    }

    /// Get a cached response for the given (method, url).
    pub async fn get(&self, method: &str, url: &str) -> Option<CachedEntry> {
        self.inner.get(&Self::cache_key(method, url)).await
    }

    /// Store a response in the cache.
    pub async fn put(&self, method: &str, url: &str, entry: CachedEntry) {
        self.inner.insert(Self::cache_key(method, url), entry).await;
    }

    /// Invalidate a specific (method, url) entry.
    pub async fn invalidate(&self, method: &str, url: &str) {
        self.inner.invalidate(&Self::cache_key(method, url)).await;
    }

    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }
}
```

- [ ] **Step 4: 更新现有 request_cache 测试调用**

修改 `src/crawl/runtime/request_cache.rs` 内现有 4 个测试，给 put/get/invalidate 加 method 参数。例如 `test_cache_put_and_get`：

```rust
    #[tokio::test]
    async fn test_cache_put_and_get() {
        let cache = RequestCache::new(100, Duration::from_secs(60));
        let entry = CachedEntry {
            status: 200,
            headers: HashMap::from([("content-type".to_string(), "text/html".to_string())]),
            body: b"<html>hello</html>".to_vec(),
        };
        cache.put("GET", "https://example.com", entry.clone()).await;

        let got = cache.get("GET", "https://example.com").await;
        assert!(got.is_some());
        let got = got.unwrap();
        assert_eq!(got.status, 200);
        assert_eq!(got.body, b"<html>hello</html>");
    }
```

对其余 3 个测试（`test_cache_miss`、`test_cache_invalidate`、`test_cache_entry_count`）同样加 `"GET"` 参数。

- [ ] **Step 5: 更新 engine.rs 调用处**

修改 `src/crawl/engine.rs` 的 `process_request`。先定义 method_str（已有，L161-166，但定义在缓存查询之后）。把 method_str 提前到 RequestCache 查询之前。

当前 L142-157（RequestCache 查询）在 L161（method_str 定义）之前。调整顺序：把 method_str 定义移到 L141 之前。

```rust
    // 1.85. 提前计算 method_str（RequestCache 查询需要）
    let method_str = match req.method {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
    };

    // 2. 内存缓存检查 (RequestCache) — 键含 method
    if let Some(ref rc) = ctx.request_cache {
        if let Some(entry) = rc.get(method_str, &req.url).await {
            let resp = SpiderResponse {
                url: req.url.clone(),
                status: entry.status,
                headers: entry.headers,
                body: entry.body,
                request: req.clone(),
                tracker: None,
                from_cache: true,
            };
            stats.cache_hits.fetch_add(1, Ordering::SeqCst);
            record_status(stats, resp.status).await;
            return process_response(ctx, resp, &req).await;
        }
    }
```

删除原 L161-166 的 method_str 定义（已上移）。保留 dev_mode SQLite 缓存段（L167-237）使用已有的 method_str。

修改 RequestCache 写入（L241-250）：

```rust
        // 7.5. 写入 RequestCache
        if let Some(ref rc) = ctx.request_cache {
            if let Some(ref resp) = final_resp {
                rc.put(method_str, &req.url, super::request_cache::CachedEntry {
                    status: resp.status,
                    headers: resp.headers.clone(),
                    body: resp.body.clone(),
                }).await;
            }
        }
```

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo build 2>&1 | tail -10`
Expected: 编译通过（所有 RequestCache 调用点已更新）。

Run: `cargo test --lib crawl::runtime::request_cache 2>&1 | tail -10`
Expected: PASS（含新测试 + 现有 4 个）。

Run: `cargo test --test unified_fetcher_test 2>&1 | tail -10`（若有用 RequestCache）
Expected: 通过。

- [ ] **Step 7: Commit**

```bash
git add src/crawl/runtime/request_cache.rs src/crawl/engine.rs
git commit -m "fix(cache): RequestCache 键含 HTTP 方法

- get/put/invalidate 新增 method 参数，键为 \"{method} {url}\"
- engine.rs 调用处传入 method_str，与 dev_mode SQLite 缓存一致
- 修复 POST 与 GET 同 URL 共享缓存返回错误响应的问题"
```

---

