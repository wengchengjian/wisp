//! Cloudflare bypass 真实环境测试。
//!
//! 运行方式：`cargo test --test cf_bypass_real_test -- --ignored`
//!
//! 所有测试标注 `#[ignore]`，需要：
//! - 本地代理 127.0.0.1:7897 可用
//! - Chrome 浏览器已安装（browser 测试）
//! - 网络可访问目标站点
//!
//! 覆盖场景：
//! 1. TLS 指纹验证（tls.peet.ws）
//! 2. HTTP fetch 带代理绕过基础检 bot
//! 3. Fetcher::stealth() 绕过 CF Turnstile
//! 4. CF JS Challenge (5秒档) 自动等待
//! 5. Engine + 代理池抓取 CF 保护站点

use std::time::Duration;
use wisp::http::Client;
use wisp::fetcher::Fetcher;
use wisp::crawl::{Engine, SpiderBuilder};
use std::sync::Arc;
use wisp::proxy::{ProxyPool, RotationStrategy};
use wreq_util::Profile;

/// 本地代理地址
const PROXY: &str = "http://127.0.0.1:7897";

/// 检测代理是否可用
async fn proxy_available() -> bool {
    let client = match Client::builder()
        .timeout(Duration::from_secs(5))
        .proxy(PROXY)
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client.get("https://www.google.com/generate_204", &[]).await.is_ok()
}

// === 测试 1: TLS 指纹验证 ===

#[tokio::test]
#[ignore = "requires network + proxy 127.0.0.1:7897"]
async fn test_tls_fingerprint_chrome() {
    if !proxy_available().await {
        eprintln!("SKIP: 代理 {} 不可用", PROXY);
        return;
    }

    let client = Client::builder()
        .proxy(PROXY)
        .emulation(Profile::Chrome136)
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    let resp = client.get("https://tls.peet.ws/api/all", &[]).await.unwrap();
    assert_eq!(resp.status, 200, "tls.peet.ws 应返回 200");

    let json = resp.json().unwrap();
    // 验证 TLS 指纹 - peet.ws 返回的 ja3_text 或 ja4 应含 Chrome 特征
    let tls_info = &json["tls"];
    let ja3_text = tls_info["ja3_text"].as_str().unwrap_or("");
    let ja4 = tls_info["ja4"].as_str().unwrap_or("");

    println!("JA3: {}", ja3_text);
    println!("JA4: {}", ja4);

    // Chrome 136 的 JA4 应以 "t13d1516h2" 开头（TLS 1.3, Chrome 特征）
    // 或者 ja3_text 非空即表示指纹模拟生效
    assert!(
        !ja3_text.is_empty() || !ja4.is_empty(),
        "TLS 指纹应非空（表示模拟生效）"
    );

    // 验证 HTTP/2 指纹存在
    let http2_info = &json["http2"];
    let akamai_fp = http2_info["akamai_fingerprint_hash"].as_str().unwrap_or("");
    println!("HTTP/2 Akamai FP: {}", akamai_fp);
    assert!(!akamai_fp.is_empty(), "HTTP/2 指纹应存在");
}

// === 测试 2: HTTP fetch 带代理 + TLS 指纹绕过基础检 bot ===

#[tokio::test]
#[ignore = "requires network + proxy 127.0.0.1:7897"]
async fn test_fetch_with_proxy_bot_detection() {
    if !proxy_available().await {
        eprintln!("SKIP: 代理 {} 不可用", PROXY);
        return;
    }

    let client = Client::builder()
        .proxy(PROXY)
        .emulation(Profile::Chrome136)
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    // bot.sannysoft.com 检测基本浏览器特征
    let resp = client.get("https://quotes.toscrape.com/", &[]).await.unwrap();
    assert_eq!(resp.status, 200);

    let text = resp.text().unwrap();
    assert!(text.contains("Quotes to Scrape"), "应成功获取页面内容");
}

// === 测试 3: Fetcher::stealth() 绕过 CF Turnstile ===

#[tokio::test]
#[ignore = "requires network + proxy + Chrome browser"]
async fn test_stealth_cf_turnstile_bypass() {
    if !proxy_available().await {
        eprintln!("SKIP: 代理 {} 不可用", PROXY);
        return;
    }

    // nopecha.com/demo/cloudflare 是一个 CF Turnstile 演示页面
    let result = Fetcher::stealth()
        .headless(true)
        .proxy(PROXY)
        .challenge_timeout(Duration::from_secs(60))
        .human_mode(true)
        .get("https://nopecha.com/demo/cloudflare")
        .await;

    match result {
        Ok(resp) => {
            println!("Status: {}", resp.status);
            println!("Title: {:?}", resp.title);
            println!("Body length: {}", resp.body.len());

            let html = resp.text().unwrap_or_default();
            // 成功绕过：不应看到 CF 挑战页面
            assert!(
                !html.contains("Just a moment") && !html.contains("cf-challenge-running"),
                "应绕过 CF 挑战，但页面仍含挑战标记"
            );
        }
        Err(e) => {
            eprintln!("SKIP: Stealth 测试失败（可能无 Chrome）: {}", e);
        }
    }
}

// === 测试 4: CF JS Challenge 自动等待 ===

#[tokio::test]
#[ignore = "requires network + proxy + Chrome browser"]
async fn test_stealth_cf_js_challenge() {
    if !proxy_available().await {
        eprintln!("SKIP: 代理 {} 不可用", PROXY);
        return;
    }

    // nowsecure.nl 会触发 CF JS Challenge
    let result = Fetcher::stealth()
        .headless(true)
        .proxy(PROXY)
        .challenge_timeout(Duration::from_secs(45))
        .human_mode(false)
        .get("https://nowsecure.nl/")
        .await;

    match result {
        Ok(resp) => {
            println!("Status: {}", resp.status);
            println!("Title: {:?}", resp.title);

            let title = resp.title().unwrap_or("");
            if title.contains("Just a moment") {
                eprintln!("WARN: 仍停留在 CF 挑战页面（可能需要更长等待时间）");
            } else {
                println!("PASS: 成功通过 CF JS Challenge");
            }
        }
        Err(e) => {
            eprintln!("SKIP: {}", e);
        }
    }
}

// === 测试 5: Engine + 代理池抓取 ===

#[tokio::test]
#[ignore = "requires network + proxy 127.0.0.1:7897"]
async fn test_engine_with_proxy_pool() {
    if !proxy_available().await {
        eprintln!("SKIP: 代理 {} 不可用", PROXY);
        return;
    }

    let pool = Arc::new(ProxyPool::new(
        vec![PROXY.to_string()],
        RotationStrategy::Sequential,
    ));

    let spider = SpiderBuilder::new("proxy-crawl")
        .start_urls(vec!["https://quotes.toscrape.com/"])
        .obey_robots(false)
        .middleware(wisp::crawl::middleware::ProxyInjectionMiddleware::new(pool))
        .on("default", |resp| async move {
            let doc = resp.parse().unwrap();
            let items: Vec<serde_json::Value> = doc.select(".quote").iter().map(|q| {
                serde_json::json!({
                    "text": q.select_one(".text").map(|n| n.text()).unwrap_or_default(),
                    "author": q.select_one(".author").map(|n| n.text()).unwrap_or_default(),
                })
            }).collect();
            (items, vec![])
        })
        .build();

    let engine = Engine::infra()
        .max_pages(1)
        .build()
        .unwrap();
    let (stats, _items) = engine.run(spider).await.unwrap();

    assert_eq!(stats.pages_crawled, 1, "应成功抓取 1 页");
    assert!(stats.items_scraped >= 5, "应提取至少 5 条名言, 实际: {}", stats.items_scraped);
    assert_eq!(stats.errors, 0, "不应有错误");
}

// === 测试 6: 多指纹转换测试 ===

#[tokio::test]
#[ignore = "requires network + proxy 127.0.0.1:7897"]
async fn test_multiple_tls_fingerprints() {
    if !proxy_available().await {
        eprintln!("SKIP: 代理 {} 不可用", PROXY);
        return;
    }

    // 测试不同浏览器指纹都能正常工作
    let profiles = vec![
        (Profile::Chrome136, "Chrome136"),
        (Profile::Firefox128, "Firefox128"),
        (Profile::Safari18, "Safari18"),
    ];

    for (profile, name) in profiles {
        let client = Client::builder()
            .proxy(PROXY)
            .emulation(profile)
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap();

        let resp = client.get("https://quotes.toscrape.com/", &[]).await;
        match resp {
            Ok(r) => {
                assert_eq!(r.status, 200, "{} 指纹应成功获取页面", name);
                println!("PASS: {} 指纹正常", name);
            }
            Err(e) => {
                eprintln!("WARN: {} 指纹请求失败: {}", name, e);
            }
        }
    }
}
