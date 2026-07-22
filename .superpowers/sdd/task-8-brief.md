# Task 8: Engine 重构为 buffer_unordered 并发（修正版）

**Files:**
- Modify: `src/crawl/mod.rs`（重写 Engine struct + impl + fetch_page）
- Create: `tests/crawl_concurrency_test.rs`

**注意：** 本 brief 修正了原 plan 中的 3 个编译问题：
1. `self.spider` 部分移动后访问 `self.config` → 提前提取所有 config 值
2. skip 路径返回 `()` vs future 类型不一致 → 所有逻辑放入单个 `async move` 块
3. `start_urls()` 在 `Arc::new(self.spider)` 后调用 → 提前提取

- [ ] **Step 1: 创建 tests/crawl_concurrency_test.rs**

```rust
//! Verify Spider Engine respects max_concurrent limit.

use std::sync::Arc;
use async_trait::async_trait;
use wisp::crawl::{Spider, SpiderRequest, SpiderResponse, Engine};
use serde_json::Value;

struct ConcurrencySpider;

#[async_trait]
impl Spider for ConcurrencySpider {
    fn name(&self) -> &str { "concurrency-test" }
    fn start_urls(&self) -> Vec<String> {
        // 10 URLs that each take 100ms to respond
        (0..10).map(|i| format!("https://httpbin.org/delay/0.1?i={}", i)).collect()
    }
    fn concurrent_requests(&self) -> u32 { 4 }
    async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        (vec![], vec![])
    }
    async fn on_start(&self) {}
    async fn on_close(&self) {}
}

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_max_concurrent_respected() {
    let spider = ConcurrencySpider;
    let stats = Engine::new(spider)
        .max_pages(10)
        .run()
        .await
        .unwrap();
    // Smoke test: should complete without panic
    assert_eq!(stats.pages_crawled, 10);
}
```

- [ ] **Step 2: 重写 src/crawl/mod.rs 的 Engine 部分**

**保留不变的部分：** `SpiderRequest`, `SpiderResponse`, `Spider` trait, `Method`, `CrawlStats`, 以及顶部的模块声明和 imports。

**替换：** `Engine` struct + `impl Engine<S>` + `fetch_page` 函数。

在文件顶部的 imports 区域，确保有以下 imports（追加到现有 use 语句之后）：

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::collections::HashMap;
use futures::stream::{self, StreamExt};
use tokio::sync::Mutex;
```

注意：现有文件已有 `use std::collections::{HashMap, HashSet};` 和 `use std::time::Duration;` 等。需要合并，不要重复 import。具体来说：
- `HashMap` 已在 `use std::collections::{HashMap, HashSet};` 中导入
- `Duration` 已导入
- 需要新增：`AtomicUsize`, `Ordering`, `Arc`, `futures::stream`, `tokio::sync::Mutex`

替换 Engine struct 和 impl 块（从 `/// The crawling engine` 到文件末尾的 `}`）为：

```rust
/// Engine configuration.
pub struct EngineConfig {
    pub max_pages: usize,
    pub max_concurrent: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self { max_pages: 1000, max_concurrent: 8 }
    }
}

/// The crawling engine that drives a Spider.
pub struct Engine<S: Spider> {
    spider: S,
    config: EngineConfig,
}

impl<S: Spider> Engine<S> {
    pub fn new(spider: S) -> Self {
        let max_concurrent = spider.concurrent_requests() as usize;
        Self {
            spider,
            config: EngineConfig {
                max_concurrent,
                ..Default::default()
            },
        }
    }

    pub fn max_pages(mut self, n: usize) -> Self { self.config.max_pages = n; self }
    pub fn max_concurrent(mut self, n: usize) -> Self { self.config.max_concurrent = n; self }

    pub async fn run(self) -> Result<CrawlStats> {
        let start = std::time::Instant::now();
        // 提前提取所有需要的信息（避免 self 部分移动问题）
        let max_pages = self.config.max_pages;
        let max_concurrent = self.config.max_concurrent;
        let obey_robots = self.spider.obey_robots();
        let allowed = self.spider.allowed_domains();
        let start_urls = self.spider.start_urls();
        let fetcher_config = self.spider.fetcher_config();

        let client = Client::builder()
            .timeout(fetcher_config.timeout)
            .build()?;

        self.spider.on_start().await;

        let spider = Arc::new(self.spider);
        let sched = Arc::new(scheduler::Scheduler::new());
        let robots_cache = Arc::new(Mutex::new(robots::RobotsCache::new()));

        // Seed start URLs
        for url in start_urls {
            sched.push(SpiderRequest::get(&url)).await;
        }

        // Channel for follow requests 回灌
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<SpiderRequest>();
        let stats_items = Arc::new(AtomicUsize::new(0));
        let stats_pages = Arc::new(AtomicUsize::new(0));
        let stats_errors = Arc::new(AtomicUsize::new(0));

        // Domain semaphores for per-domain throttling
        let domain_sems: Arc<Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let follow_rx = Arc::new(Mutex::new(follow_rx));
        let client = Arc::new(client);
        let allowed = Arc::new(allowed);

        let mut stream = {
            let sched = sched.clone();
            let follow_rx = follow_rx.clone();
            let follow_tx = follow_tx.clone();
            let spider = spider.clone();
            let client = client.clone();
            let stats_pages = stats_pages.clone();
            let stats_errors = stats_errors.clone();
            let stats_items = stats_items.clone();
            let domain_sems = domain_sems.clone();
            let robots_cache = robots_cache.clone();
            let allowed = allowed.clone();

            stream::unfold((), move |_| {
                let sched = sched.clone();
                let follow_rx = follow_rx.clone();
                let follow_tx = follow_tx.clone();
                let spider = spider.clone();
                let client = client.clone();
                let stats_pages = stats_pages.clone();
                let stats_errors = stats_errors.clone();
                let stats_items = stats_items.clone();
                let domain_sems = domain_sems.clone();
                let robots_cache = robots_cache.clone();
                let allowed = allowed.clone();

                async move {
                    // 1. Drain follow channel into scheduler
                    let mut rx_guard = follow_rx.lock().await;
                    while let Ok(req) = rx_guard.try_recv() {
                        sched.push(req).await;
                    }
                    drop(rx_guard);

                    // 2. Check page budget
                    if stats_pages.load(Ordering::SeqCst) >= max_pages {
                        return None;
                    }

                    // 3. Pop next request
                    let req = sched.pop().await?;

                    // 4-7. All logic in a single async block (unified future type)
                    let spider_clone = spider.clone();
                    let stats_pages_c = stats_pages.clone();
                    let stats_errors_c = stats_errors.clone();
                    let stats_items_c = stats_items.clone();
                    let follow_tx_c = follow_tx.clone();
                    let client_c = client.clone();
                    let domain_sems_c = domain_sems.clone();
                    let robots_cache_c = robots_cache.clone();
                    let allowed_c = allowed.clone();

                    let fut = async move {
                        // 4. Domain filter
                        if !allowed_c.is_empty() {
                            if let Ok(parsed) = url::Url::parse(&req.url) {
                                if let Some(host) = parsed.host_str() {
                                    if !allowed_c.contains(host) {
                                        return;  // skip
                                    }
                                }
                            }
                        }

                        // 5. Robots check
                        if obey_robots {
                            let url_clone = req.url.clone();
                            let client_r = client_c.clone();
                            let allowed = {
                                let rc = robots_cache_c.lock().await;
                                rc.is_allowed(&client_r, &url_clone).await
                            };
                            if !allowed {
                                return;
                            }
                        }

                        // 6. Per-domain throttle
                        let domain = url::Url::parse(&req.url)
                            .ok()
                            .and_then(|u| u.host_str().map(|s| s.to_string()))
                            .unwrap_or_default();
                        let sem = {
                            let mut sems = domain_sems_c.lock().await;
                            sems.entry(domain.clone())
                                .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent)))
                                .clone()
                        };
                        let _permit = sem.acquire_owned().await.unwrap();

                        // 7. Fetch
                        match fetch_page(&client_c, &req).await {
                            Ok(resp) => {
                                if spider_clone.is_blocked(&resp) {
                                    stats_errors_c.fetch_add(1, Ordering::SeqCst);
                                    return;
                                }
                                stats_pages_c.fetch_add(1, Ordering::SeqCst);
                                let (items, follows) = spider_clone.parse(resp).await;
                                for item in items {
                                    if let Some(_processed) = spider_clone.on_item(item).await {
                                        stats_items_c.fetch_add(1, Ordering::SeqCst);
                                    }
                                }
                                for f in follows {
                                    let _ = follow_tx_c.send(f);
                                }
                            }
                            Err(e) => {
                                stats_errors_c.fetch_add(1, Ordering::SeqCst);
                                spider_clone.on_error(&req, &e.to_string()).await;
                            }
                        }
                    };

                    // Return the future for buffer_unordered
                    Some((fut, ()))
                }
            })
            .map(|(fut, _)| fut)
            .buffer_unordered(max_concurrent)
        };

        // Drive the stream to completion
        while stream.next().await.is_some() {}

        spider.on_close().await;

        Ok(CrawlStats {
            items_scraped: stats_items.load(Ordering::SeqCst),
            pages_crawled: stats_pages.load(Ordering::SeqCst),
            errors: stats_errors.load(Ordering::SeqCst),
            duration: start.elapsed(),
        })
    }
}

async fn fetch_page(client: &Client, req: &SpiderRequest) -> Result<SpiderResponse> {
    let resp = match req.method {
        Method::Get => client.get(&req.url).await?,
        Method::Post => client.post(&req.url, req.body.as_deref(), None).await?,
        Method::Put => client.put(&req.url, req.body.as_deref(), None).await?,
        Method::Delete => client.delete(&req.url).await?,
    };

    Ok(SpiderResponse {
        url: resp.url.clone(),
        status: resp.status,
        headers: resp.headers.clone(),
        body: resp.body.clone(),
        request: req.clone(),
    })
}
```

- [ ] **Step 3: 运行 cargo check 验证编译**

Run: `cargo check`
Expected: 编译通过。如果有编译错误，根据错误信息修复（常见问题见下）。

常见编译问题及修复：
1. `RobotsCache::is_allowed` 需要 `&mut self`，但 `rc` 是 `MutexGuard` — `MutexGuard` 实现了 `DerefMut`，所以 `rc.is_allowed(...)` 应该能工作。如果不行，尝试 `(&mut *rc).is_allowed(...)`。
2. `atomic` imports 重复 — 确保只在文件顶部 import 一次
3. `HashSet` 已在现有 imports 中 — 不要重复

- [ ] **Step 4: 运行已有测试确保未破坏**

Run: `cargo test --lib`
Expected: 现有测试通过

- [ ] **Step 5: 提交**

```bash
git add src/crawl/mod.rs tests/crawl_concurrency_test.rs
git commit -m "refactor: Engine 重构为 buffer_unordered 真并发 + per-domain 信号量"
```
