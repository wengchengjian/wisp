//! 诊断测试：固定同一 cookie/UA/sec-ch-ua，对比不同 emulation 档位在 HTTP 模式下
//! 的差异，并打印 wreq 实际构建的请求 headers，定位 403 根因。
//!
//! 运行：cd f:/project/wisp && cargo nextest run -p wisp-fetcher --features stealth --test cf_http_diagnose -- --nocapture

use std::time::Duration;

use wisp_http::{Client, Config};
use wreq_util::Profile;

/// 本地代理（与浏览器签发 cf_clearance 时相同）。
const PROXY: &str = "http://127.0.0.1:7897";
/// bz444 首页。
const TARGET: &str = "https://www.bz444444444.com/";

/// 从 cf_sessions.json 加载会话：返回 (cookie, ua, sec_ch_ua)。
///
/// cookie 存于 `.bz444444444.com`（父域）下，UA/sec_ch_ua 存于 `www.` 下，
/// 这里从所有 session 里分别取第一个非空值。
fn load_session() -> Option<(String, String, String)> {
    let content =
        std::fs::read_to_string("f:/project/banzhu-rs/wisp-data/cf_sessions.json").ok()?;
    let map: serde_json::Value = serde_json::from_str(&content).ok()?;
    let mut cookie = String::new();
    let mut ua = String::new();
    let mut schua = String::new();
    for sess in map.as_object()?.values() {
        if let Some(cookies) = sess["cookies"].as_array() {
            for c in cookies {
                if c["name"].as_str() == Some("cf_clearance") {
                    cookie = format!("cf_clearance={}", c["value"].as_str().unwrap_or_default());
                }
            }
        }
        if ua.is_empty() {
            ua = sess["ua"].as_str().unwrap_or("").to_string();
        }
        if schua.is_empty() {
            schua = sess["sec_ch_ua"].as_str().unwrap_or("").to_string();
        }
    }
    if cookie.is_empty() {
        None
    } else {
        Some((cookie, ua, schua))
    }
}

fn build_headers(cookie: &str, ua: &str, schua: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![
        ("User-Agent".into(), ua.into()),
        ("sec-ch-ua-mobile".into(), "?0".into()),
        ("sec-ch-ua-platform".into(), "\"Windows\"".into()),
        (
            "accept".into(),
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7".into(),
        ),
        ("accept-language".into(), "zh-CN,zh;q=0.9,en;q=0.8".into()),
        ("upgrade-insecure-requests".into(), "1".into()),
        ("sec-fetch-site".into(), "none".into()),
        ("sec-fetch-mode".into(), "navigate".into()),
        ("sec-fetch-user".into(), "?1".into()),
        ("sec-fetch-dest".into(), "document".into()),
        ("Cookie".into(), cookie.into()),
    ];
    if let Some(s) = schua {
        headers.push(("sec-ch-ua".into(), s.into()));
    }
    headers
}

async fn try_emulation(
    name: &str,
    profile: Option<Profile>,
    cookie: &str,
    ua: &str,
    schua: Option<&str>,
) {
    let mut cfg = Config {
        timeout: Duration::from_secs(30),
        proxy: Some(PROXY.to_string()),
        ..Default::default()
    };
    if let Some(p) = profile {
        cfg.emulation = Some(p);
    }
    let client = match Client::from_config(cfg) {
        Ok(c) => c,
        Err(e) => {
            println!("[{name}] 构建 client 失败: {e}");
            return;
        }
    };
    let headers = build_headers(cookie, ua, schua);
    match client.get(TARGET, &headers).await {
        Ok(resp) => {
            let text = resp.text().unwrap_or_default();
            let title = text
                .split("<title>")
                .nth(1)
                .and_then(|s| s.split("</title>").next())
                .unwrap_or("")
                .to_string();
            let preview: String = text.chars().take(120).collect();
            println!(
                "[{name}] status={} title={:?} len={} preview={:?}",
                resp.status,
                title,
                text.len(),
                preview
            );
        }
        Err(e) => println!("[{name}] 请求失败: {e}"),
    }
}

/// 直接用 wreq 构建 Request，打印最终 headers（验证 emulation 默认头与手动头的合并）。
fn inspect_wreq_headers(name: &str, profile: Option<Profile>, ua: &str, schua: Option<&str>) {
    let builder = wreq::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(wreq::redirect::Policy::none());
    let builder = if let Some(p) = profile {
        builder.emulation(p)
    } else {
        builder
    };
    let Ok(client) = builder.build() else {
        return;
    };
    let mut rb = client.get(TARGET);
    let extra = build_headers("", ua, schua);
    for (k, v) in extra {
        rb = rb.header(k, v);
    }
    if let Ok(req) = rb.build() {
        println!("[{name}] 请求 headers:");
        for (k, v) in req.headers() {
            println!("    {k}: {}", String::from_utf8_lossy(v.as_bytes()));
        }
    } else {
        println!("[{name}] 构建 Request 失败");
    }
}

#[tokio::test]
async fn diagnose_cf_http_reuse() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();

    let Some((cookie, ua, schua)) = load_session() else {
        println!("!! 未找到 cf_clearance cookie");
        return;
    };
    let schua_opt = if schua.is_empty() {
        None
    } else {
        Some(schua.as_str())
    };
    println!("cookie: {}...", &cookie[..40.min(cookie.len())]);
    println!("ua: {ua}");
    println!("sec_ch_ua: {schua}");

    println!("\n=== 1. Chrome149 指纹 + 真实会话头 ===");
    try_emulation(
        "Chrome149",
        Some(Profile::Chrome149),
        &cookie,
        &ua,
        schua_opt,
    )
    .await;

    println!("\n=== 2. Chrome148 指纹 + 真实会话头 ===");
    try_emulation(
        "Chrome148",
        Some(Profile::Chrome148),
        &cookie,
        &ua,
        schua_opt,
    )
    .await;

    println!("\n=== 3. Chrome136 指纹 + 真实会话头 ===");
    try_emulation(
        "Chrome136",
        Some(Profile::Chrome136),
        &cookie,
        &ua,
        schua_opt,
    )
    .await;

    println!("\n=== 4. 无 emulation + 真实会话头 ===");
    try_emulation("no-emulation", None, &cookie, &ua, schua_opt).await;

    println!("\n=== 5. wreq 请求头对比（Chrome148 vs Chrome149）===");
    inspect_wreq_headers("Chrome148", Some(Profile::Chrome148), &ua, schua_opt);
    println!();
    inspect_wreq_headers("Chrome149", Some(Profile::Chrome149), &ua, schua_opt);
    println!();
    inspect_wreq_headers("no-emulation", None, &ua, schua_opt);
}
