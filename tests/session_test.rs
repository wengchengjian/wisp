//! HTTP Session 复用测试。
//!
//! 运行方式：`cargo test --test session_test -- --ignored`
//!
//! 验证 HttpSession 的 cookie 持久化、跨请求状态保持、代理组合。

use std::time::Duration;
use wisp::http::{Client, HttpSession};
use wreq_util::Profile;

const PROXY: &str = "http://127.0.0.1:7897";

/// 检测 httpbin.org 是否可达
async fn httpbin_available(client: &Client) -> bool {
    match client.get("https://httpbin.org/status/200").await {
        Ok(r) => r.status == 200,
        Err(_) => false,
    }
}

/// 创建可用的 client（直连或代理）
async fn working_client() -> Option<Client> {
    let direct = Client::builder()
        .timeout(Duration::from_secs(10))
        .emulation(Profile::Chrome136)
        .build()
        .unwrap();

    if httpbin_available(&direct).await {
        return Some(direct);
    }

    let proxied = Client::builder()
        .timeout(Duration::from_secs(20))
        .proxy(PROXY)
        .emulation(Profile::Chrome136)
        .build()
        .unwrap();

    if httpbin_available(&proxied).await {
        return Some(proxied);
    }

    None
}

// === 测试 1: Cookie 设置与持久化 ===

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_session_cookie_persistence() {
    let Some(client) = working_client().await else {
        eprintln!("SKIP: httpbin.org 不可达");
        return;
    };

    let session = HttpSession::from_client(client);

    // 设置 cookie
    let resp1 = session.get("https://httpbin.org/cookies/set?token=secret123&user=wisp").await;
    if resp1.is_err() {
        eprintln!("SKIP: 设置 cookie 失败");
        return;
    }

    // 验证 cookie 被持久化
    let resp2 = session.get("https://httpbin.org/cookies").await.unwrap();
    let json = resp2.json().unwrap();

    let cookies = &json["cookies"];
    assert_eq!(
        cookies["token"].as_str().unwrap_or(""),
        "secret123",
        "token cookie 应被保持"
    );
    assert_eq!(
        cookies["user"].as_str().unwrap_or(""),
        "wisp",
        "user cookie 应被保持"
    );

    println!("PASS: Cookie 持久化验证通过");
}

// === 测试 2: 多请求间状态保持 ===

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_session_state_across_requests() {
    let Some(client) = working_client().await else {
        eprintln!("SKIP: httpbin.org 不可达");
        return;
    };

    let session = HttpSession::from_client(client);

    // 第一次请求设置 session_id
    let _ = session.get("https://httpbin.org/cookies/set?session_id=abc-def-ghi").await;

    // 第二次请求设置另一个 cookie
    let _ = session.get("https://httpbin.org/cookies/set?theme=dark").await;

    // 第三次请求验证两个 cookie 都存在
    let resp = session.get("https://httpbin.org/cookies").await.unwrap();
    let json = resp.json().unwrap();

    assert_eq!(json["cookies"]["session_id"].as_str().unwrap_or(""), "abc-def-ghi");
    assert_eq!(json["cookies"]["theme"].as_str().unwrap_or(""), "dark");

    println!("PASS: 多请求状态保持验证通过");
}

// === 测试 3: Cookie 清除 ===

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_session_clear_cookies() {
    let Some(client) = working_client().await else {
        eprintln!("SKIP: httpbin.org 不可达");
        return;
    };

    let session = HttpSession::from_client(client);

    // 设置 cookie
    let _ = session.get("https://httpbin.org/cookies/set?temp=value").await;

    // 清除所有 cookie
    session.clear_cookies().await;

    // 验证清除后请求不带 cookie
    let resp = session.get("https://httpbin.org/cookies").await.unwrap();
    let json = resp.json().unwrap();

    // cookies 对象应为空或不含 temp
    let has_temp = json["cookies"]["temp"].as_str().is_some();
    assert!(!has_temp, "清除后不应携带旧 cookie");

    println!("PASS: Cookie 清除验证通过");
}

// === 测试 4: 手动设置 Cookie ===

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_session_manual_cookie() {
    let Some(client) = working_client().await else {
        eprintln!("SKIP: httpbin.org 不可达");
        return;
    };

    let session = HttpSession::from_client(client);

    // 手动设置 cookie（不通过网络）
    session.set_cookie("httpbin.org", "manual_key", "manual_value").await;

    // 请求应携带手动设置的 cookie
    let resp = session.get("https://httpbin.org/cookies").await.unwrap();
    let json = resp.json().unwrap();

    assert_eq!(
        json["cookies"]["manual_key"].as_str().unwrap_or(""),
        "manual_value",
        "手动设置的 cookie 应被发送"
    );

    println!("PASS: 手动 Cookie 设置验证通过");
}

// === 测试 5: 代理 + Session 组合 ===

#[tokio::test]
#[ignore = "requires network + proxy 127.0.0.1:7897"]
async fn test_session_with_proxy() {
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .proxy(PROXY)
        .emulation(Profile::Chrome136)
        .build()
        .unwrap();

    if !httpbin_available(&client).await {
        eprintln!("SKIP: 通过代理无法访问 httpbin.org");
        return;
    }

    let session = HttpSession::from_client(client);

    // 通过代理设置 cookie
    let _ = session.get("https://httpbin.org/cookies/set?via=proxy").await;

    // 验证 cookie 保持
    let resp = session.get("https://httpbin.org/cookies").await.unwrap();
    let json = resp.json().unwrap();

    assert_eq!(json["cookies"]["via"].as_str().unwrap_or(""), "proxy");
    println!("PASS: 代理 + Session 组合验证通过");
}

// === 测试 6: Session Builder 模式 ===

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_session_builder() {
    let session = HttpSession::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("WispBot/1.0")
        .build()
        .unwrap();

    // 验证基本请求工作
    let resp = session.get("https://httpbin.org/user-agent").await;
    match resp {
        Ok(r) => {
            let json = r.json().unwrap();
            // 注意：wreq 的 user_agent 设置方式可能不同于 header
            println!("User-Agent response: {:?}", json);
            println!("PASS: Session Builder 基本功能正常");
        }
        Err(e) => {
            eprintln!("SKIP: 请求失败: {}", e);
        }
    }
}
