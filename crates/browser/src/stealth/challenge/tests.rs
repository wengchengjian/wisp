use super::*;

// === ChallengeType 枚举单元测试 ===

#[test]
fn test_challenge_type_equality() {
    assert_eq!(ChallengeType::None, ChallengeType::None);
    assert_eq!(ChallengeType::JsChallenge, ChallengeType::JsChallenge);
    assert_eq!(ChallengeType::Turnstile, ChallengeType::Turnstile);
    assert_eq!(
        ChallengeType::ManagedChallenge,
        ChallengeType::ManagedChallenge
    );
    assert_ne!(ChallengeType::None, ChallengeType::JsChallenge);
    assert_ne!(ChallengeType::Turnstile, ChallengeType::ManagedChallenge);
}

#[test]
fn test_challenge_type_clone_copy() {
    let ct = ChallengeType::Turnstile;
    let cloned = ct;
    let copied = ct; // Copy
    assert_eq!(ct, cloned);
    assert_eq!(ct, copied);
}

#[test]
fn test_challenge_type_debug() {
    assert_eq!(format!("{:?}", ChallengeType::None), "None");
    assert_eq!(format!("{:?}", ChallengeType::JsChallenge), "JsChallenge");
    assert_eq!(format!("{:?}", ChallengeType::Turnstile), "Turnstile");
    assert_eq!(
        format!("{:?}", ChallengeType::ManagedChallenge),
        "ManagedChallenge"
    );
}

// === 集成测试（需要 Chrome 环境） ===

/// 测试 ChallengeSolver::detect() 在普通页面上返回 None。
/// 需要本地 Chrome/Chromium 环境。
#[tokio::test]
#[ignore = "需要 Chrome 浏览器环境"]
async fn test_detect_no_challenge_on_normal_page() {
    use crate::Browser;
    use wisp_core::config::LaunchOptions;

    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .expect("启动浏览器");

    let mut page = browser.new_page().await.expect("创建页面");
    // data: URL 不触发任何 CF 挑战
    page.goto("data:text/html,<html><body><h1>Hello</h1></body></html>")
        .await
        .expect("导航");

    let solver = ChallengeSolver::new(&page);
    let result = solver.detect().await.expect("detect 应成功");
    assert_eq!(result, ChallengeType::None, "普通页面不应检测到挑战");

    let _ = page.close().await;
    browser.close().await.expect("关闭浏览器");
}

/// 测试 interactive Turnstile 配置被识别为 Turnstile。
#[tokio::test]
#[ignore = "需要 Chrome 浏览器环境"]
async fn test_detect_interactive_turnstile_marker() {
    use crate::Browser;
    use wisp_core::config::LaunchOptions;

    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .expect("启动浏览器");

    let mut page = browser.new_page().await.expect("创建页面");
    page.goto("data:text/html,<html><body><script>window._cf_chl_opt={cType: 'interactive'};</script></body></html>")
        .await
        .expect("导航");

    let solver = ChallengeSolver::new(&page);
    let result = solver.detect().await.expect("detect 应成功");
    assert_eq!(
        result,
        ChallengeType::Turnstile,
        "interactive CF 配置应识别为 Turnstile"
    );

    let _ = page.close().await;
    browser.close().await.expect("关闭浏览器");
}

/// 测试 is_cloudflare_page 在普通页面上返回 false。
#[tokio::test]
#[ignore = "需要 Chrome 浏览器环境"]
async fn test_is_cloudflare_page_normal() {
    use crate::Browser;
    use wisp_core::config::LaunchOptions;

    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .expect("启动浏览器");

    let mut page = browser.new_page().await.expect("创建页面");
    page.goto("data:text/html,<html><body><p>Normal content</p></body></html>")
        .await
        .expect("导航");

    let result = is_cloudflare_page(&page).await.expect("检测应成功");
    assert!(!result, "普通页面不应被误判为 CF 挑战页");

    let _ = page.close().await;
    browser.close().await.expect("关闭浏览器");
}

/// 测试包含 CF 特有标识的页面被正确检测。
#[tokio::test]
#[ignore = "需要 Chrome 浏览器环境"]
async fn test_detect_js_challenge_markers() {
    use crate::Browser;
    use wisp_core::config::LaunchOptions;

    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .expect("启动浏览器");

    let mut page = browser.new_page().await.expect("创建页面");
    // 模拟 JS Challenge 页面（包含特有标识）
    page.goto("data:text/html,<html><head><title>Just a moment...</title></head><body><div id='challenge-running'>Checking...</div></body></html>")
        .await
        .expect("导航");

    let solver = ChallengeSolver::new(&page);
    let result = solver.detect().await.expect("detect 应成功");
    assert_eq!(result, ChallengeType::JsChallenge, "应检测到 JS Challenge");

    let _ = page.close().await;
    browser.close().await.expect("关闭浏览器");
}
