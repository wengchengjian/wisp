### Task 3: P1-1b proxy_clients 改用 DashMap

**Files:**
- Modify: `src/crawl/engine.rs:66,638,668,695-705,800`
- Modify: `src/crawl/runner.rs:225`
- Test: `tests/p1_proxy_clients_test.rs`（新建）

**Interfaces:**
- Produces: `EngineShared.proxy_clients: Arc<DashMap<String, Arc<Client>>>`（原 `Arc<Mutex<HashMap<...>>>`）。
- Produces: `fetch_page` / `fetch_page_inner` 参数 `proxy_clients: &DashMap<String, Arc<Client>>`。

- [ ] **Step 1: 写失败测试 — 相同 proxy 只构建一次 Client**

新建 `tests/p1_proxy_clients_test.rs`：

```rust
//! P1-1b: proxy_clients 用 DashMap，相同 proxy 复用 Client。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use wisp::crawl::engine::fetch_page_inner;
use wisp::crawl::{SpiderRequest, Method};
use wisp::fetcher::FetchMode;
use wisp::http::{Client, Config};

#[tokio::test]
async fn proxy_clients_caches_client_per_proxy_url() {
    // proxy_clients 暴露为 DashMap，验证相同 proxy 两次 fetch 只产生一个缓存条目
    let client = Arc::new(Client::builder().build().unwrap());
    let config = Config::default();
    let proxy_clients = Arc::new(dashmap::DashMap::new());
    let req = SpiderRequest::get("http://127.0.0.1:1/unreachable");

    // 两次 fetch 同一 proxy（连接会失败，但 Client 应被缓存）
    for _ in 0..2 {
        let _ = fetch_page_inner(
            &client,
            &req,
            Some("http://127.0.0.1:1"),
            FetchMode::Http,
            &config,
            &proxy_clients,
        ).await;
    }

    assert_eq!(proxy_clients.len(), 1, "相同 proxy 应只缓存 1 个 Client");
    assert!(proxy_clients.contains_key("http://127.0.0.1:1"));
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --test p1_proxy_clients_test`
Expected: 编译失败 — `fetch_page_inner` 不可见（`pub(crate)`），`proxy_clients` 类型不匹配（当前是 `Mutex`）。

- [ ] **Step 3: 修改 engine.rs — proxy_clients 字段类型**

`src/crawl/engine.rs:66` 当前：

```rust
    pub proxy_clients: Arc<Mutex<HashMap<String, Arc<Client>>>>,
```

替换为：

```rust
    pub proxy_clients: Arc<dashmap::DashMap<String, Arc<Client>>>,
```

- [ ] **Step 4: 修改 fetch_page 与 fetch_page_inner 签名**

`src/crawl/engine.rs:631-638` `fetch_page` 签名末参：

```rust
    proxy_clients: &Mutex<HashMap<String, Arc<Client>>>,
```

替换为：

```rust
    proxy_clients: &dashmap::DashMap<String, Arc<Client>>,
```

`src/crawl/engine.rs:668` `fetch_page_inner` 签名同参同样替换。

并将两个函数从 `pub(crate) async fn` 改为 `pub async fn`（供集成测试访问）。即 `src/crawl/engine.rs:631` 的 `pub(crate) async fn fetch_page(` → `pub async fn fetch_page(`，`src/crawl/engine.rs:661` 的 `pub(crate) async fn fetch_page_inner(` → `pub async fn fetch_page_inner(`。

- [ ] **Step 5: 修改 fetch_page_inner 内部锁逻辑**

`src/crawl/engine.rs:693-705` 当前：

```rust
    // Http 模式
    // 代理 Client 缓存：相同 proxy URL 复用已建立的连接，避免每请求 TLS 握手
    let proxy_client: Option<Arc<Client>> = if let Some(proxy) = proxy_url {
        let mut cache = proxy_clients.lock().await;
        if !cache.contains_key(proxy) {
            let new_client = Client::builder()
                .timeout(client.config_ref().timeout)
                .proxy(proxy)
                .build()?;
            cache.insert(proxy.to_string(), Arc::new(new_client));
        }
        Some(cache.get(proxy).unwrap().clone())
    } else {
        None
    };
```

替换为（DashMap：快路径 get，慢路径 build 后 entry::or_insert，错误向上传播）：

```rust
    // Http 模式
    // 代理 Client 缓存：相同 proxy URL 复用已建立的连接，避免每请求 TLS 握手
    let proxy_client: Option<Arc<Client>> = if let Some(proxy) = proxy_url {
        if let Some(c) = proxy_clients.get(proxy) {
            Some(c.clone())
        } else {
            // 慢路径：构建新 client（可能失败，错误向上传播）
            let new_client = Client::builder()
                .timeout(client.config_ref().timeout)
                .proxy(proxy)
                .build()?;
            let arc = Arc::new(new_client);
            // 并发安全：若另一 task 已插入，用已存在的；否则用新建的
            Some(proxy_clients.entry(proxy.to_string()).or_insert(arc).clone())
        }
    } else {
        None
    };
```

- [ ] **Step 6: 修改 runner.rs 构造**

`src/crawl/runner.rs:225` 当前：

```rust
                proxy_clients: Arc::new(Mutex::new(HashMap::new())),
```

替换为：

```rust
                proxy_clients: Arc::new(dashmap::DashMap::new()),
```

- [ ] **Step 7: 修改 engine.rs make_ctx 测试辅助**

`src/crawl/engine.rs:800` 当前：

```rust
                proxy_clients: Arc::new(Mutex::new(HashMap::new())),
```

替换为：

```rust
                proxy_clients: Arc::new(dashmap::DashMap::new()),
```

- [ ] **Step 8: 暴露 fetch_page_inner 供集成测试**

`src/crawl/mod.rs` re-export 区追加（紧接 Task 2 的 `pub use engine::record_status;`）：

```rust
pub use engine::{record_status, fetch_page, fetch_page_inner};
```

（若 Task 2 已加 `pub use engine::record_status;`，此处合并为 `pub use engine::{record_status, fetch_page, fetch_page_inner};`。）

- [ ] **Step 9: 运行测试验证通过**

Run: `cargo test --test p1_proxy_clients_test && cargo test --lib && cargo build`
Expected: 新测试 PASS；lib 206 测试全绿；编译无错。

- [ ] **Step 10: 提交**

```bash
git add src/crawl/engine.rs src/crawl/runner.rs src/crawl/mod.rs tests/p1_proxy_clients_test.rs
git commit -m "perf: proxy_clients 改用 DashMap 消除全局锁 (P1-1b)"
```

---

