//! 验证 Page drop 后是否泄漏 Chrome tab。
//!
//! 测试逻辑：
//! 1. 启动 Browser
//! 2. 创建一个常驻「探针 page」，用它调用 `Target.getTargets` 获取基线 page 数量
//! 3. 循环 N 次：创建 page 并立即 drop（作用域结束）
//! 4. 再次用同一个探针 page 查询 `Target.getTargets`
//! 5. 比较两次的 page-type target 数量：
//!    - 不泄漏：差值 ≈ 0（容忍 ≤1 调度延迟）
//!    - 泄漏：差值 ≈ N
//!
//! 探针 page 在两次测量中都被算入，差异抵消，避免探针自身污染结果。

use std::path::PathBuf;
use serde_json::json;
use wisp::{Browser, LaunchOptions};

async fn launch_browser() -> Option<Browser> {
    // 优先用环境变量指定的 Chrome 路径（便于在 WSL 下指向 Windows Chrome）
    let executable_path = std::env::var("CHROME_PATH").ok().map(PathBuf::from);

    // 若使用 Windows Chrome（路径含 .exe），user_data_dir 需用 Windows 风格路径
    // 否则用 Linux /tmp 风格
    let is_windows_exe = executable_path
        .as_ref()
        .map(|p| p.to_string_lossy().contains(".exe"))
        .unwrap_or(false);

    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let user_data = if is_windows_exe {
        // Windows Chrome 需要 Windows 风格路径，写到 Windows TEMP
        let win_temp = std::env::var("WIN_TEMP").unwrap_or_else(|_| "C:\\Windows\\Temp".into());
        PathBuf::from(format!("{}\\wisp-tab-leak-{}", win_temp, suffix))
    } else {
        std::env::temp_dir().join(format!("wisp-tab-leak-{}-{suffix}", std::process::id()))
    };

    let result = Browser::launch(LaunchOptions {
        headless: true,
        executable_path,
        user_data_dir: Some(user_data),
        ..Default::default()
    })
    .await;
    if let Err(e) = &result {
        eprintln!("launch failed: {e:?}");
    }
    result.ok()
}

/// 用探针 page 查询当前所有 type=="page" 的 target 数量。
async fn count_page_targets(probe: &wisp::Page) -> usize {
    let result = probe
        .cmd("Target.getTargets", json!({}))
        .await
        .expect("Target.getTargets should succeed");
    let targets = result
        .get("targetInfos")
        .and_then(|v| v.as_array())
        .expect("targetInfos should be an array");
    targets
        .iter()
        .filter(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
        .count()
}

#[tokio::test]
async fn page_drop_does_not_leak_tabs() {
    let Some(browser) = launch_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    // 常驻探针 page：在两次测量间保持存活，自身贡献抵消
    let mut probe = browser.new_page().await.expect("probe new_page");
    // 探针导航到 about:blank 避免干扰
    let _ = probe.goto("about:blank").await;

    let baseline = count_page_targets(&probe).await;
    println!("Baseline page targets (incl. probe): {baseline}");

    // 创建并 drop 5 个 page
    for i in 0..5 {
        {
            let _page = browser.new_page().await.expect("temp new_page");
            // _page 在此作用域结束 drop
            println!("created & dropped page #{i}");
        }
    }

    // 给 CDP 一点时间处理任何异步清理（如果有的话）
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let after = count_page_targets(&probe).await;
    println!("After 5 drops: page targets (incl. probe): {after}");

    let diff = after.saturating_sub(baseline);
    assert!(
        diff <= 1,
        "Tab leak detected: baseline={baseline}, after={after}, diff={diff} (expected ≤1, got {} leaked)",
        diff
    );

    let _ = browser.close().await;
}
