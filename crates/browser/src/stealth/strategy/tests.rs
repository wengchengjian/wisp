use super::*;

#[test]
fn test_from_config_default() {
    let config = StealthConfig::default();
    let cookie_jar: Arc<dyn CookieJar> = Arc::new(wisp_core::cookie::MockCookieJar::new());
    let strategy = StealthStrategy::from_config(&config, cookie_jar);
    assert_eq!(strategy.challenge_timeout, config.challenge_timeout);
    assert_eq!(strategy.human_mode, config.human_mode);
    assert_eq!(strategy.wait_for, config.wait_for);
    assert_eq!(strategy.extra_wait_ms, config.extra_wait_ms);
}

#[test]
fn test_from_config_custom() {
    let config = StealthConfig {
        human_mode: false,
        challenge_timeout: Duration::from_secs(60),
        wait_for: Some(".loaded".to_string()),
        extra_wait_ms: 2000,
        ..Default::default()
    };
    let cookie_jar: Arc<dyn CookieJar> = Arc::new(wisp_core::cookie::MockCookieJar::new());
    let strategy = StealthStrategy::from_config(&config, cookie_jar);
    assert!(!strategy.human_mode);
    assert_eq!(strategy.challenge_timeout, Duration::from_secs(60));
    assert_eq!(strategy.wait_for.as_deref(), Some(".loaded"));
    assert_eq!(strategy.extra_wait_ms, 2000);
}

/// 集成测试：CF 挑战解决 + cookie 持久化。
/// 运行方式：cargo test --lib browser::stealth::strategy -- --ignored
#[tokio::test]
#[ignore = "需要 CF 保护的站点环境"]
async fn test_stealth_strategy_solves_cf() {
    use crate::BrowserPool;
    use wisp_core::Request;
    use wisp_core::config::LaunchOptions;

    let config = StealthConfig::default();
    let cookie_jar: Arc<dyn CookieJar> = Arc::new(wisp_core::cookie::MockCookieJar::new());
    let strategy = StealthStrategy::from_config(&config, cookie_jar);

    let pool = BrowserPool::new(1, LaunchOptions::default());
    let mut handle = pool.acquire().await.expect("acquire page");
    let page = handle.page_mut();

    // 替换为实际的 CF 保护站点
    let req = Request::get("https://example.com/");
    let resp = strategy.fetch(page, &req).await.expect("fetch 应成功");
    assert_eq!(resp.status, 200);
}
