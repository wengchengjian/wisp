//! 验证单 Browser 多 Page 并发模型的正确性。
//!
//! 旧模型（多 Browser 进程）有索引移位和句柄别名 bug；新模型（单 Browser
//! + Semaphore）无索引概念，这些 bug 不可能发生。本文件验证新模型的核心
//! 保证：并发 acquire 的 page 互相独立，permit 正确释放。
//!
//! `Browser::launch` 需要真实 Chrome，故本文件全部测试标记 `#[ignore]`，
//! 仅在有 Chrome 的环境手动运行：`cargo test --test cr_fix_pool_test -- --ignored`。

use std::path::PathBuf;
use std::time::Duration;

use wisp::browser::BrowserPool;
use wisp::LaunchOptions;

/// 从 CHROME_PATH 环境变量构造 LaunchOptions（测试用）。
fn launch_options() -> LaunchOptions {
    let executable_path = std::env::var("CHROME_PATH").ok().map(PathBuf::from);
    LaunchOptions {
        executable_path,
        ..Default::default()
    }
}

/// 验证并发 acquire 的 page 互相独立（不同 session_id）。
///
/// 新模型下，每次 acquire 创建新 tab，tab 之间有独立的 CDP session。
/// 如果 session 共享，会导致事件串扰（A tab 的事件被 B tab 收到）。
#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn concurrent_pages_have_independent_sessions() {
    let pool = BrowserPool::new(3, launch_options());

    let h1 = pool.acquire().await.expect("acquire h1");
    let h2 = pool.acquire().await.expect("acquire h2");
    let h3 = pool.acquire().await.expect("acquire h3");

    let s1 = h1.page().session_id().to_string();
    let s2 = h2.page().session_id().to_string();
    let s3 = h3.page().session_id().to_string();

    assert_ne!(s1, s2, "page 1 和 page 2 的 session_id 必须不同");
    assert_ne!(s2, s3, "page 2 和 page 3 的 session_id 必须不同");
    assert_ne!(s1, s3, "page 1 和 page 3 的 session_id 必须不同");

    pool.shutdown().await;
}

/// 验证 permit 在 handle drop 后释放，允许新的 acquire。
///
/// 场景（max_concurrent_pages=2）：
/// 1. acquire h1, h2 → permits=0
/// 2. drop h1 → permits=1
/// 3. acquire h3 → 应立即成功（不阻塞）
#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn permit_releases_on_handle_drop() {
    let pool = BrowserPool::new(2, launch_options());

    let h1 = pool.acquire().await.expect("acquire h1");
    let _h2 = pool.acquire().await.expect("acquire h2");
    assert_eq!(pool.available_permits(), 0);

    drop(h1);
    // permit 应立即释放（OwnedSemaphorePermit::Drop 是同步的）
    assert_eq!(pool.available_permits(), 1);

    // h3 应立即成功，不需等待
    let _h3 = tokio::time::timeout(Duration::from_secs(5), pool.acquire())
        .await
        .expect("acquire h3 should not timeout")
        .expect("acquire h3");

    assert_eq!(pool.available_permits(), 0);

    pool.shutdown().await;
}

/// 验证并发达上限时 acquire 阻塞，直到有 handle 释放。
#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn acquire_blocks_when_permits_exhausted() {
    let pool = BrowserPool::new(1, launch_options());

    let h1 = pool.acquire().await.expect("acquire h1");
    assert_eq!(pool.available_permits(), 0);

    // 起 task 尝试 acquire（应阻塞）
    let pool_clone = pool.clone();
    let acquire_task = tokio::spawn(async move {
        pool_clone.acquire().await.expect("acquire h2")
    });

    // 等待 200ms，确认 task 仍在阻塞（未完成）
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!acquire_task.is_finished(), "acquire should block when permits=0");

    // 释放 h1，task 应完成
    drop(h1);
    let _h2 = tokio::time::timeout(Duration::from_secs(5), acquire_task)
        .await
        .expect("acquire should complete after release")
        .expect("acquire task should succeed");

    pool.shutdown().await;
}
