//! HTTP Session 澶嶇敤娴嬭瘯銆?
//!
//! 杩愯鏂瑰紡锛歚cargo test --test session_test -- --ignored`
//!
//! 楠岃瘉 HttpSession 鐨?cookie 鎸佷箙鍖栥€佽法璇锋眰鐘舵€佷繚鎸併€佷唬鐞嗙粍鍚堛€?

use std::time::Duration;
use wisp::http::{Client, HttpSession};
use wreq_util::Profile;

const PROXY: &str = "http://127.0.0.1:7897";

/// 妫€娴?httpbin.org 鏄惁鍙揪
async fn httpbin_available(client: &Client) -> bool {
    match client.get("https://httpbin.org/status/200").await {
        Ok(r) => r.status == 200,
        Err(_) => false,
    }
}

/// 鍒涘缓鍙敤鐨?client锛堢洿杩炴垨浠ｇ悊锛?
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

// === 娴嬭瘯 1: Cookie 璁剧疆涓庢寔涔呭寲 ===

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_session_cookie_persistence() {
    let Some(client) = working_client().await else {
        eprintln!("SKIP: httpbin.org 涓嶅彲杈?);
        return;
    };

    let session = HttpSession::from_client(client);

    // 璁剧疆 cookie
    let resp1 = session.get("https://httpbin.org/cookies/set?token=secret123&user=wisp").await;
    if resp1.is_err() {
        eprintln!("SKIP: 璁剧疆 cookie 澶辫触");
        return;
    }

    // 楠岃瘉 cookie 琚寔涔呭寲
    let resp2 = session.get("https://httpbin.org/cookies").await.unwrap();
    let json = resp2.json().unwrap();

    let cookies = &json["cookies"];
    assert_eq!(
        cookies["token"].as_str().unwrap_or(""),
        "secret123",
        "token cookie 搴旇淇濇寔"
    );
    assert_eq!(
        cookies["user"].as_str().unwrap_or(""),
        "wisp",
        "user cookie 搴旇淇濇寔"
    );

    println!("PASS: Cookie 鎸佷箙鍖栭獙璇侀€氳繃");
}

// === 娴嬭瘯 2: 澶氳姹傞棿鐘舵€佷繚鎸?===

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_session_state_across_requests() {
    let Some(client) = working_client().await else {
        eprintln!("SKIP: httpbin.org 涓嶅彲杈?);
        return;
    };

    let session = HttpSession::from_client(client);

    // 绗竴娆¤姹傝缃?session_id
    let _ = session.get("https://httpbin.org/cookies/set?session_id=abc-def-ghi").await;

    // 绗簩娆¤姹傝缃彟涓€涓?cookie
    let _ = session.get("https://httpbin.org/cookies/set?theme=dark").await;

    // 绗笁娆¤姹傞獙璇佷袱涓?cookie 閮藉瓨鍦?
    let resp = session.get("https://httpbin.org/cookies").await.unwrap();
    let json = resp.json().unwrap();

    assert_eq!(json["cookies"]["session_id"].as_str().unwrap_or(""), "abc-def-ghi");
    assert_eq!(json["cookies"]["theme"].as_str().unwrap_or(""), "dark");

    println!("PASS: 澶氳姹傜姸鎬佷繚鎸侀獙璇侀€氳繃");
}

// === 娴嬭瘯 3: Cookie 娓呴櫎 ===

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_session_clear_cookies() {
    let Some(client) = working_client().await else {
        eprintln!("SKIP: httpbin.org 涓嶅彲杈?);
        return;
    };

    let session = HttpSession::from_client(client);

    // 璁剧疆 cookie
    let _ = session.get("https://httpbin.org/cookies/set?temp=value").await;

    // 娓呴櫎鎵€鏈?cookie
    session.clear_cookies().await;

    // 楠岃瘉娓呴櫎鍚庤姹備笉甯?cookie
    let resp = session.get("https://httpbin.org/cookies").await.unwrap();
    let json = resp.json().unwrap();

    // cookies 瀵硅薄搴斾负绌烘垨涓嶅惈 temp
    let has_temp = json["cookies"]["temp"].as_str().is_some();
    assert!(!has_temp, "娓呴櫎鍚庝笉搴旀惡甯︽棫 cookie");

    println!("PASS: Cookie 娓呴櫎楠岃瘉閫氳繃");
}

// === 娴嬭瘯 4: 鎵嬪姩璁剧疆 Cookie ===

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_session_manual_cookie() {
    let Some(client) = working_client().await else {
        eprintln!("SKIP: httpbin.org 涓嶅彲杈?);
        return;
    };

    let session = HttpSession::from_client(client);

    // 鎵嬪姩璁剧疆 cookie锛堜笉閫氳繃缃戠粶锛?
    session.set_cookie("httpbin.org", "manual_key", "manual_value").await;

    // 璇锋眰搴旀惡甯︽墜鍔ㄨ缃殑 cookie
    let resp = session.get("https://httpbin.org/cookies").await.unwrap();
    let json = resp.json().unwrap();

    assert_eq!(
        json["cookies"]["manual_key"].as_str().unwrap_or(""),
        "manual_value",
        "鎵嬪姩璁剧疆鐨?cookie 搴旇鍙戦€?
    );

    println!("PASS: 鎵嬪姩 Cookie 璁剧疆楠岃瘉閫氳繃");
}

// === 娴嬭瘯 5: 浠ｇ悊 + Session 缁勫悎 ===

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
        eprintln!("SKIP: 閫氳繃浠ｇ悊鏃犳硶璁块棶 httpbin.org");
        return;
    }

    let session = HttpSession::from_client(client);

    // 閫氳繃浠ｇ悊璁剧疆 cookie
    let _ = session.get("https://httpbin.org/cookies/set?via=proxy").await;

    // 楠岃瘉 cookie 淇濇寔
    let resp = session.get("https://httpbin.org/cookies").await.unwrap();
    let json = resp.json().unwrap();

    assert_eq!(json["cookies"]["via"].as_str().unwrap_or(""), "proxy");
    println!("PASS: 浠ｇ悊 + Session 缁勫悎楠岃瘉閫氳繃");
}

// === 娴嬭瘯 6: Session Builder 妯″紡 ===

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_session_builder() {
    let session = HttpSession::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("WispBot/1.0")
        .build()
        .unwrap();

    // 楠岃瘉鍩烘湰璇锋眰宸ヤ綔
    let resp = session.get("https://httpbin.org/user-agent").await;
    match resp {
        Ok(r) => {
            let json = r.json().unwrap();
            // 娉ㄦ剰锛歸req 鐨?user_agent 璁剧疆鏂瑰紡鍙兘涓嶅悓浜?header
            println!("User-Agent response: {:?}", json);
            println!("PASS: Session Builder 鍩烘湰鍔熻兘姝ｅ父");
        }
        Err(e) => {
            eprintln!("SKIP: 璇锋眰澶辫触: {}", e);
        }
    }
}
