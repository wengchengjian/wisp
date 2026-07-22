//! StopCondition 终止策略单元测试。
//!
//! 验证 MaxPages 停止条件的判定逻辑。真实多 Spider E2E 测试为后续任务。

use std::time::Duration;
use wisp::crawl::{MaxPages, StopCondition, StopContext};

#[test]
fn test_max_pages_condition() {
    // 不实际跑爬虫，只验证 StopCondition 逻辑
    let cond = MaxPages(50);
    let ctx = StopContext { pages: 50, items: 0, errors: 0, in_flight: 0, elapsed: Duration::ZERO, queue_size: 0 };
    assert!(cond.should_stop(&ctx));
}
