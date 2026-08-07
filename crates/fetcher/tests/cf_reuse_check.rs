//! 临时验证测试：用已保存的 CF session（wisp-data/cf_sessions.json）走 HTTP 快速路径
//! 访问目标站点，验证 HTTP 复用 cf_clearance 是否能通过（200），还是被 CF 拒绝（403/None）。
//!
//! 运行：cd f:/project/wisp && cargo test -p wisp-fetcher --test cf_reuse_check -- --nocapture

use std::time::Duration;

use wisp_fetcher::{FetchClient, FetchClientConfig, FetchMode, Request};
use wisp_http::Config as HttpConfig;

#[tokio::test]
async fn check_cf_reuse_on_bz444() {
    // 使用 banzhu 的 cf_sessions.json（含 bz444 的 cf_clearance）
    let config = FetchClientConfig {
        cf_data_dir: std::path::PathBuf::from("f:/project/banzhu-rs/wisp-data"),
        http: HttpConfig {
            timeout: Duration::from_secs(30),
            // cf_clearance 绑定签发时的出口 IP：浏览器通过 127.0.0.1:7897 签发，
            // HTTP 复用必须走同一代理，否则 CF 判定 IP 不匹配而 403。
            proxy: Some("http://127.0.0.1:7897".to_string()),
            ..Default::default()
        },
        // cf_sessions.json 里的 cookie 可能已保存一段时间；调大 TTL 以加载未过期的旧 cookie，
        // 验证「父域匹配 + 指纹对齐」后 HTTP 复用是否真正通过。
        // cf_clearance 本身未过期（expires 是未来时间），但 TTL 需 > saved_at_age 才能加载。
        cf_cookie_ttl: Duration::from_secs(12 * 3600), // 12 小时，确保加载
        ..Default::default()
    };

    // 初始化 tracing，让 try_http_with_session_cookie 打印实际 cookie/UA/status
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .try_init();

    // 直接检查 cf_sessions.json 里 bz444 的 session 是否在 TTL 内
    let content = std::fs::read_to_string("f:/project/banzhu-rs/wisp-data/cf_sessions.json")
        .expect("读取 cf_sessions.json");
    let map: serde_json::Value = serde_json::from_str(&content).unwrap();
    let now = chrono::Utc::now().timestamp();
    for (domain, sess) in map.as_object().unwrap() {
        let saved_at = sess["saved_at"].as_i64().unwrap_or(0);
        let age = now - saved_at;
        let n_cookies = sess["cookies"].as_array().map(|a| a.len()).unwrap_or(0);
        println!(
            "session: {} saved_at_age={}s cookies={}",
            domain, age, n_cookies
        );
    }

    let fc = FetchClient::new(config).expect("创建 FetchClient");

    for url in [
        "https://www.bz444444444.com/",
        "https://www.bz444444444.com/shuku/0-lastupdate-0-1.html",
    ] {
        let req = Request::get(url);
        println!("=== 尝试 HTTP 复用: {url} ===");
        match fc.try_http_with_session_cookie(&req).await {
            Ok(Some(resp)) => {
                println!(">>> 通过! status={}", resp.status);
                let text = resp.text().unwrap_or_default();
                let preview: String = text.chars().take(300).collect();
                println!(">>> body 预览: {preview}");
            }
            Ok(None) => {
                println!(">>> 未通过 (403/非200)，HTTP 复用失败");
            }
            Err(e) => {
                println!(">>> 请求出错: {e}");
            }
        }
    }

    // 对照：普通 HTTP（无 CF cookie）
    let plain_req = Request::get("https://www.bz444444444.com/");
    println!("=== 对照: 普通 HTTP（无 CF cookie）===");
    match fc.fetch(&plain_req, FetchMode::Http).await {
        Ok(resp) => println!(">>> 普通 HTTP status={}", resp.status),
        Err(e) => println!(">>> 普通 HTTP 出错: {e}"),
    }
}
