//! 验证 BrowserPool 修复后不出现句柄别名（bug #1）与索引移位（bug #2）。
//!
//! `Browser::launch` 需要真实 Chrome，故本文件全部测试标记 `#[ignore]`，
//! 仅在有 Chrome 的环境手动运行：`cargo test --test cr_fix_pool_test -- --ignored`。
//!
//! 纯逻辑（不依赖 Chrome）的索引正确性测试见 `src/browser/pool.rs` 的
//! `pick_idle_slot_*` 单元测试。

use std::sync::Arc;
use std::time::Duration;

use wisp::browser::{BrowserPool, CdpSession};
use wisp::LaunchOptions;

/// bug #1 复现：release-then-reacquire 路径下旧 `position(|p| p.in_use)` 返回错误索引。
///
/// 场景（max_size=2）：
/// 1. acquire h1 → 新建槽 0，h1.index=0
/// 2. acquire h2 → 新建槽 1，h2.index=1（两个均 in_use）
/// 3. drop h2    → 槽 1 标记空闲
/// 4. acquire h3 → 复用槽 1，标记 in_use
///
/// 旧实现：`position(|p| p.in_use)` 返回 0（第一个 in_use），h3.index=0，
/// 与 h1 别名（指向同一 browser session）。
/// 修复后：`enumerate` 捕获正确索引 1，h3 与 h1 指向不同 session。
#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn acquire_does_not_alias_after_release_and_reacquire() {
    let pool = BrowserPool::new(2, Duration::from_secs(300), LaunchOptions::default());

    let h1 = pool.acquire().await.expect("acquire h1");
    let h2 = pool.acquire().await.expect("acquire h2");
    // 释放 h2，使槽 1 变空闲
    drop(h2);
    // 给后台 release task 一点时间
    tokio::time::sleep(Duration::from_millis(50)).await;

    let h3 = pool.acquire().await.expect("acquire h3");

    let r1 = h1.browser_ref().await.expect("h1 ref");
    let r3 = h3.browser_ref().await.expect("h3 ref");
    let s1: *const CdpSession = Arc::as_ptr(&r1.session);
    let s3: *const CdpSession = Arc::as_ptr(&r3.session);
    assert_ne!(
        s1, s3,
        "h3 不得别名 h1：release+reacquire 后应返回不同 browser 实例"
    );

    pool.shutdown().await;
}

/// bug #2 复现：超时回收后索引不应左移。
///
/// 场景（max_size=3，idle_timeout 极短）：
/// 1. acquire h1（槽 0）、acquire h2（槽 1）
/// 2. drop h1 → 槽 0 空闲；等待超过 idle_timeout
/// 3. acquire h3 → 触发超时回收槽 0
///
/// 旧实现：`retain` 移除槽 0 后，槽 1 左移到索引 0，h2.index=1 越界/错位。
/// 修复后：`take()` 清空槽 0 内容但保留空槽，h2.index=1 仍指向原 browser。
#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn retain_does_not_shift_indices_after_timeout() {
    let pool = BrowserPool::new(3, Duration::from_millis(100), LaunchOptions::default());

    let h1 = pool.acquire().await.expect("acquire h1");
    let h2 = pool.acquire().await.expect("acquire h2");
    let h2_session: *const CdpSession = Arc::as_ptr(&h2.browser_ref().await.expect("h2 ref").session);

    // 释放 h1，等待超时
    drop(h1);
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 触发一次 acquire，内部会回收超时的槽 0
    let _h3 = pool.acquire().await.expect("acquire h3");

    // h2 的索引必须仍指向原来的 browser（不应因 retain 移位而错位）
    let h2_ref = h2.browser_ref().await.expect("h2 ref 仍有效");
    let h2_session_after: *const CdpSession = Arc::as_ptr(&h2_ref.session);
    assert_eq!(
        h2_session, h2_session_after,
        "超时回收后 h2 索引不得左移，必须仍指向原 browser"
    );

    pool.shutdown().await;
}
