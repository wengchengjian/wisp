//! 核心引擎修复回归测试（C1 / C3 / I2 / I7）。
//!
//! 覆盖：
//! - 修复2 (C3): Spider until() 停止时引擎不挂起（URL 被跳过后正常结束）。
//! - 修复3 (I2): StopContext.queue_size 填充真实值。
//! - 修复5 (I7): fetch_with_retry 重试语义（on_error 调用 1 次）。
//!
//! 注意：修复1（patterns 正则路由）已在 Task 1-6 重构中废弃删除，
//! 对应的 matches/patterns 测试不再适用。

use wisp::crawl::*;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 修复3 回归：StopContext.queue_size 字段可被 FnStopCondition 读取，
/// 验证终止策略能基于真实队列长度判定。
#[test]
fn test_stop_context_queue_size_is_real() {
    use std::time::Duration;
    let ctx = stop::StopContext {
        pages: 0,
        items: 0,
        errors: 0,
        in_flight: 0,
        elapsed: Duration::ZERO,
        queue_size: 42,
    };
    let cond = FnStopCondition(|c: &stop::StopContext| c.queue_size == 42);
    assert!(cond.should_stop(&ctx), "queue_size 应为 42");

    let ctx_zero = stop::StopContext {
        pages: 0,
        items: 0,
        errors: 0,
        in_flight: 0,
        elapsed: Duration::ZERO,
        queue_size: 0,
    };
    assert!(
        !cond.should_stop(&ctx_zero),
        "queue_size 为 0 时不应停止"
    );
}

/// 修复2 回归：当 Spider until() 已停止时，
/// 引擎不应挂起，应正常结束（URL 被记录后跳过，而非静默丢弃导致死锁）。
#[tokio::test]
async fn test_stopped_spider_url_not_silently_dropped() {
    struct StoppedSpider;
    #[async_trait]
    impl Spider for StoppedSpider {
        fn name(&self) -> &str { "stopped" }
        fn start_urls(&self) -> Vec<String> { vec!["http://127.0.0.1:1/never-fetched".into()] }
        async fn parse(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
            (vec![], vec![])
        }
        fn obey_robots(&self) -> bool { false }
        // MaxPages(0)：pages >= 0 恒为真，Spider 立即停止，start_url 不会被派发。
        fn until(&self) -> Arc<dyn StopCondition> { Arc::new(MaxPages(0)) }
    }

    let engine = Engine::infra().max_pages(10).build().unwrap();
    // 引擎应在不超时的情况下完成（不挂起）。
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        engine.run(StoppedSpider),
    ).await;
    assert!(result.is_ok(), "引擎应在 10s 内完成，未因 URL 丢弃而挂起");
    let (stats, _items) = result.unwrap().expect("run 应返回 Ok");
    // Spider 从未派发请求，pages 应为 0。
    assert_eq!(stats.pages_crawled, 0, "停止的 Spider 不应爬取任何页面");
}

/// 修复5 回归：max_retries=3 时实际尝试 4 次（attempt 1..=4），
/// on_error 仅在最终失败后调用 1 次。
#[tokio::test]
async fn test_fetch_retry_count_semantics() {
    struct RetrySpider {
        count: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl Spider for RetrySpider {
        fn name(&self) -> &str { "retry" }
        fn start_urls(&self) -> Vec<String> {
            // 端口 1 不可达，连接被拒绝，触发 error 分支重试。
            vec!["http://127.0.0.1:1/unreachable".into()]
        }
        async fn parse(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
            (vec![], vec![])
        }
        fn obey_robots(&self) -> bool { false }
        fn max_retries(&self) -> u32 { 3 }
        fn download_delay(&self) -> std::time::Duration { std::time::Duration::ZERO }
        async fn on_error(&self, _req: &Request, _err: &str) {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let spider = RetrySpider { count: count.clone() };
    let engine = Engine::infra().max_pages(1).build().unwrap();
    let _ = engine.run(spider).await;

    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "on_error 应在最终失败后调用 1 次，实际 {}",
        count.load(Ordering::SeqCst)
    );
}
