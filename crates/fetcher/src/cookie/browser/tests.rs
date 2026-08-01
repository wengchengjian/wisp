use super::*;
use crate::cookie::Cookie;
use crate::cookie::CookieJar;
use serde_json::json;
use std::sync::Arc;
use url::Url;

#[test]
fn value_to_cookie_extracts_fields() {
    let v = json!({
        "name": "session",
        "value": "abc",
        "domain": "example.com",
        "path": "/",
        "secure": true,
        "httpOnly": true,
        "sameSite": "Lax",
        "expires": 1234567890.0,
    });
    let cookie = BrowserCookieJar::value_to_cookie(&v, "fallback").expect("完整字段应解析成功");
    assert_eq!(cookie.name, "session");
    assert_eq!(cookie.value, "abc");
    assert_eq!(cookie.domain, "example.com");
    assert_eq!(cookie.path, "/");
    assert!(cookie.secure);
    assert!(cookie.http_only);
    assert_eq!(cookie.same_site.as_deref(), Some("Lax"));
    assert_eq!(cookie.expires, Some(1234567890.0));
}

#[test]
fn value_to_cookie_uses_default_domain_when_missing() {
    let v = json!({
        "name": "x",
        "value": "y",
    });
    let cookie =
        BrowserCookieJar::value_to_cookie(&v, "default.com").expect("仅 name/value 也应解析");
    assert_eq!(cookie.domain, "default.com");
    assert_eq!(cookie.path, "/");
    assert!(!cookie.secure);
    assert!(!cookie.http_only);
    assert!(cookie.same_site.is_none());
    assert!(cookie.expires.is_none());
}

#[test]
fn value_to_cookie_returns_none_for_missing_name() {
    let v = json!({ "value": "y" });
    assert!(BrowserCookieJar::value_to_cookie(&v, "x").is_none());
}

// === 集成测试（需要 Chrome 环境） ===

#[tokio::test]
#[ignore = "需要 Chrome 浏览器环境"]
async fn browser_set_and_get_cookie_roundtrip() {
    use wisp_browser::Browser;
    use wisp_core::config::LaunchOptions;

    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .expect("启动浏览器");

    let mut page = browser.new_page().await.expect("创建页面");
    // 先导航到一个真实 URL 让 cookie 有 domain 上下文
    page.goto("data:text/html,<html></html>")
        .await
        .expect("导航");

    let jar =
        BrowserCookieJar::new_for_target(Arc::clone(page.session()), page.session_id().to_string());
    jar.set(Cookie {
        name: "test_cookie".into(),
        value: "value123".into(),
        domain: "localhost".into(),
        path: "/".into(),
        secure: false,
        http_only: false,
        same_site: Some("Lax".into()),
        expires: None,
    })
    .await;

    let url = Url::parse("http://localhost/").expect("合法 URL");
    let cookies = jar.get(&url).await;
    assert!(cookies
        .iter()
        .any(|c| c.name == "test_cookie" && c.value == "value123"));

    let _ = page.close().await;
    browser.close().await.expect("关闭浏览器");
}

#[tokio::test]
#[ignore = "需要 Chrome 浏览器环境"]
async fn browser_clear_removes_cookies() {
    use wisp_browser::Browser;
    use wisp_core::config::LaunchOptions;

    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .expect("启动浏览器");

    let mut page = browser.new_page().await.expect("创建页面");
    page.goto("data:text/html,<html></html>")
        .await
        .expect("导航");

    let jar =
        BrowserCookieJar::new_for_target(Arc::clone(page.session()), page.session_id().to_string());
    jar.set(Cookie {
        name: "to_clear".into(),
        value: "v".into(),
        domain: "localhost".into(),
        path: "/".into(),
        secure: false,
        http_only: false,
        same_site: None,
        expires: None,
    })
    .await;

    let url = Url::parse("http://localhost/").expect("合法 URL");
    jar.clear(&url).await;
    // clearBrowserCookies 清除所有 cookie
    let cookies = jar.get(&url).await;
    assert!(cookies.iter().all(|c| c.name != "to_clear"));

    let _ = page.close().await;
    browser.close().await.expect("关闭浏览器");
}
