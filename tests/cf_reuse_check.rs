#![cfg(feature = "stealth")]
//! CF cookie HTTP 复用真实环境测试。
//!
//! 运行方式：`cargo test --features stealth --test cf_reuse_check -- --ignored`
//!
//! 所有测试标注 `#[ignore]`（默认跳过，手动 `--ignored` 运行），需要：
//! - 已保存的 CF 会话（`wisp-data/cf_sessions.json`，含 bz 站点的 `cf_clearance`）
//! - 本地代理 127.0.0.1:7897 可用（浏览器与 HTTP 须同一出口 IP，否则 cookie 无效）
//!
//! 背景：`cf_clearance` 绑定签发时的出口 IP。直连（本机 IP）复用会 403；
//! 与浏览器同一代理出口（如 Clash 7897）复用成功（200）。
//! 测试对照「直连 vs 走代理」两种出口验证该结论。

use std::time::Duration;

use wisp_core::CookieJar;
use wisp_fetcher::cookie::CfCookieJar;
use wisp_fetcher::{FetchClient, FetchClientConfig, Request};

const PROXY: &str = "http://127.0.0.1:7897";
/// 需要 banzhu 项目已运行过爬虫，生成该 CF 会话文件。
const CF_DATA: &str = "F:/project/banzhu-rs/wisp-data";

async fn run_reuse(proxy: Option<&str>, tag: &str) {
    let mut config = FetchClientConfig::default();
    config.cf_data_dir = std::path::PathBuf::from(CF_DATA);
    config.http.timeout = Duration::from_secs(30);
    config.cf_cookie_ttl = Duration::from_secs(7200);
    if let Some(p) = proxy {
        config.http.proxy = Some(p.to_string());
    }
    let fc = match FetchClient::new(config) {
        Ok(fc) => fc,
        Err(e) => {
            println!("[{tag}] FetchClient 创建失败: {e}");
            return;
        }
    };

    for url in [
        "https://www.bz444444444.com/",
        "https://www.bz444444444.com/shuku/0-lastupdate-0-1.html",
    ] {
        let req = Request::get(url);
        println!("[{tag}] === 尝试 HTTP 复用: {url} ===");
        match fc.try_http_with_session_cookie(&req).await {
            Ok(Some(resp)) => {
                println!("[{tag}] >>> 通过! status={}", resp.status);
                let text = resp.text().unwrap_or_default();
                let preview: String = text.chars().take(200).collect();
                println!("[{tag}] >>> body: {preview}");
            }
            Ok(None) => {
                println!("[{tag}] >>> 未通过 (403/非200)，HTTP 复用失败");
            }
            Err(e) => {
                println!("[{tag}] >>> 请求出错: {e}");
            }
        }
    }
}

#[tokio::test]
#[ignore = "需要真实 CF cookie + 代理 127.0.0.1:7897"]
async fn check_cf_reuse_on_bz444() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .try_init();

    // 探针：确认 jar 里的 cookie
    let jar = CfCookieJar::new(
        &std::path::PathBuf::from(CF_DATA),
        Duration::from_secs(7200),
    );
    let u = url::Url::parse("https://www.bz444444444.com/").unwrap();
    let cookies = jar.get(&u).await;
    println!(
        "[PROBE] CfCookieJar.get(www.bz444...) -> {} cookies: {:?}",
        cookies.len(),
        cookies.iter().map(|c| c.name.clone()).collect::<Vec<_>>()
    );
    println!("[PROBE] ua={:?}", jar.ua(&u).await);
    println!("[PROBE] sec_ch_ua={:?}", jar.sec_ch_ua(&u).await);

    // 对照 A：直连（cookie 出口不匹配 → 预期 403）
    run_reuse(None, "DIRECT").await;

    // 对照 B：走 7897 代理（与浏览器同出口 → 预期 200）
    run_reuse(Some(PROXY), "PROXY").await;
}
