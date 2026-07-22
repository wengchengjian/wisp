# Task 9: 多 Spider 路由 + until 终止 E2E 测试

## 目标

创建 `tests/multi_spider_test.rs`，包含多 Spider 共享队列 + 路由 + until 终止策略的骨架测试。

这是骨架测试 + StopCondition 单元验证，不跑真实 HTTP。完整 HTTP 路由测试在 Task 10 补充。

## Files

- Create: `tests/multi_spider_test.rs`

## 完整代码（来自 plan）

```rust
//! 多 Spider 共享队列 + 路由 + until 终止策略 E2E 测试。
//!
//! 场景：ListSpider 爬取列表页 50 页停，DetailSpider 消费详情 URL。

use std::time::Duration;
use async_trait::async_trait;
use serde_json::{json, Value};
use wisp::crawl::{Spider, SpiderRequest, SpiderResponse, Engine};
use wisp::crawl::{MaxPages, NeverStop};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 列表页 Spider：从 list/1 开始，产出 list/N+1 和 detail/X
struct ListSpider {
    max_page: usize,
    list_counter: Arc<AtomicUsize>,
}

#[async_trait]
impl Spider for ListSpider {
    fn name(&self) -> &str { "list" }
    fn start_urls(&self) -> Vec<String> { vec!["http://test.example/list/1".into()] }
    fn patterns(&self) -> Vec<String> { vec![r"test\.example/list/\d+".into()] }
    fn until(&self) -> Arc<dyn wisp::crawl::StopCondition> {
        Arc::new(MaxPages(self.max_page))
    }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let n = self.list_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let items = vec![json!({ "list_page": n })];
        // follow 下一页列表 + 一个详情
        let next = resp.follow(&format!("/list/{}", n + 1)).unwrap();
        let detail = resp.follow(&format!("/detail/{}", n)).unwrap();
        (items, vec![next, detail])
    }
}

/// 详情页 Spider：消费 detail URL
struct DetailSpider {
    detail_counter: Arc<AtomicUsize>,
}

#[async_trait]
impl Spider for DetailSpider {
    fn name(&self) -> &str { "detail" }
    fn start_urls(&self) -> Vec<String> { vec![] }
    fn patterns(&self) -> Vec<String> { vec![r"test\.example/detail/\d+".into()] }
    fn until(&self) -> Arc<dyn wisp::crawl::StopCondition> {
        Arc::new(NeverStop)  // 受限于上游 ListSpider
    }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let n = self.detail_counter.fetch_add(1, Ordering::SeqCst) + 1;
        (vec![json!({ "detail_page": n })], vec![])
    }
}

#[test]
fn test_max_pages_condition() {
    // 不实际跑爬虫，只验证 StopCondition 逻辑
    use wisp::crawl::StopContext;
    let cond = MaxPages(50);
    let ctx = StopContext { pages: 50, items: 0, errors: 0, in_flight: 0, elapsed: Duration::ZERO, queue_size: 0 };
    assert!(cond.should_stop(&ctx));
}
```

## 验证步骤

1. `cargo test --test multi_spider_test` — 必须通过
2. 如果编译失败，检查：
   - `SpiderResponse::follow` 方法是否存在及签名
   - import 路径是否正确（`wisp::crawl::{MaxPages, NeverStop, StopContext}` 在 Task 2 的 mod.rs 中重导出）
   - `StopContext` 字段是否匹配（pages/items/errors/in_flight/elapsed/queue_size）
   - `Spider` trait 的方法签名是否匹配（patterns/until/parse）

## Commit

PowerShell 兼容（不支持 heredoc），用 -m：
```powershell
git add tests/multi_spider_test.rs
git commit -m "test: 多 Spider 路由与 until 终止策略骨架测试"
```
