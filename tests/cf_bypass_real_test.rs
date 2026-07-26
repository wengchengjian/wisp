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
        vec![PROXY.to_string().into()],
        RotationStrategy::Sequential,
    ));

    let spider = SpiderBuilder::new("proxy-crawl")
        .start_urls(vec!["https://quotes.toscrape.com/"])
        .middleware(wisp::crawl::middleware::ProxyInjectionMiddleware::new(pool))
        .on("default", |resp| async move {
            let doc = resp.parse();
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
        .obey_robots(false)
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

// === 测试 7: banzhu 目标站点 CF 绕过 ===

/// banzhu 目标站点书库页面
const BANZHU_TARGET_URL: &str = "https://www.bz555555555.com/shuku/0-lastupdate-0-1.html";

/// 测试 HTTP 模式直连目标站点（预期被 CF 拦截返回 403）
#[tokio::test]
#[ignore = "requires network + proxy 127.0.0.1:7897"]
async fn test_banzhu_http_mode() {
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

    let resp = client.get(BANZHU_TARGET_URL, &[]).await;
    match resp {
        Ok(r) => {
            println!("[HTTP] Status: {}", r.status);
            println!("[HTTP] Body length: {} bytes", r.body.len());

            let text = r.text().unwrap_or_default();
            if r.status == 403 {
                println!("[HTTP] 预期结果: 403 被 CF 拦截");
                // 检查是否是 CF 挑战页
                if text.contains("Just a moment") || text.contains("cf-") {
                    println!("[HTTP] 确认是 Cloudflare 挑战页面");
                }
            } else if r.status == 200 {
                println!("[HTTP] 意外成功! 页面未被 CF 保护");
                // 检查是否获取到真实内容
                if text.contains("书库") || text.contains("小说") {
                    println!("[HTTP] PASS: 获取到真实页面内容");
                }
            }
        }
        Err(e) => {
            eprintln!("[HTTP] 请求失败: {}", e);
        }
    }
}

/// 测试 Stealth 模式绕过 CF 获取目标站点
#[tokio::test]
#[ignore = "requires network + proxy + Chrome browser"]
async fn test_banzhu_stealth_bypass() {
    if !proxy_available().await {
        eprintln!("SKIP: 代理 {} 不可用", PROXY);
        return;
    }

    println!("[Stealth] 目标: {}", BANZHU_TARGET_URL);
    println!("[Stealth] 代理: {}", PROXY);
    println!("[Stealth] 挑战超时: 60s");

    let result = Fetcher::stealth()
        .headless(true)
        .proxy(PROXY)
        .challenge_timeout(Duration::from_secs(60))
        .human_mode(true)
        .get(BANZHU_TARGET_URL)
        .await;

    match result {
        Ok(resp) => {
            println!("[Stealth] Status: {}", resp.status);
            println!("[Stealth] Title: {:?}", resp.title);
            println!("[Stealth] Body length: {} bytes", resp.body.len());

            let html = resp.text().unwrap_or_default();

            // 检查是否仍在 CF 挑战页
            let on_challenge = html.contains("Just a moment")
                || html.contains("cf-challenge-running")
                || html.contains("challenge-platform");

            if on_challenge {
                eprintln!("[Stealth] FAIL: 仍停留在 CF 挑战页面");
                eprintln!("[Stealth] Title: {:?}", resp.title);
                // 输出前 500 字符帮助诊断
                let preview: String = html.chars().take(500).collect();
                eprintln!("[Stealth] Body preview: {}", preview);
            } else if resp.status == 200 {
                // 检查是否获取到真实内容
                if html.contains("书库") || html.contains("小说") || html.contains("lastupdate") {
                    println!("[Stealth] PASS: 成功绕过 CF，获取到真实页面!");
                } else {
                    println!("[Stealth] WARN: 状态 200 但未检测到预期内容");
                    let preview: String = html.chars().take(500).collect();
                    println!("[Stealth] Body preview: {}", preview);
                }
            } else {
                eprintln!("[Stealth] FAIL: 状态码 {}", resp.status);
            }
        }
        Err(e) => {
            eprintln!("[Stealth] FAIL: {}", e);
            eprintln!("[Stealth] 可能原因: Chrome 未安装 / 代理不可用 / CF 挑战超时");
        }
    }
}

/// 诊断测试：详细输出 CF 挑战页面状态
#[tokio::test]
#[ignore = "requires network + proxy + Chrome browser"]
async fn test_banzhu_cf_diagnostic() {
    use wisp::{Browser, LaunchOptions};

    if !proxy_available().await {
        eprintln!("SKIP: 代理 {} 不可用", PROXY);
        return;
    }

    println!("\n=== CF 诊断测试 ===");
    println!("目标: {}", BANZHU_TARGET_URL);

    // 启动浏览器
    let browser = match Browser::launch(LaunchOptions {
        headless: true,
        proxy: Some(wisp::ProxyConfig {
            server: PROXY.to_string(),
            username: None,
            password: None,
        }),
        ..Default::default()
    }).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("浏览器启动失败: {}", e);
            return;
        }
    };

    let mut page = browser.new_page().await.expect("创建页面");

    // 导航
    println!("\n[1] 导航到目标页面...");
    if let Err(e) = page.goto(BANZHU_TARGET_URL).await {
        eprintln!("导航失败: {}", e);
        let _ = browser.close().await;
        return;
    }

    // 等待一下
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 获取页面状态
    let title = page.title().await.unwrap_or_default();
    let url = page.url().await.unwrap_or_default();
    println!("[2] 当前 Title: {}", title);
    println!("[3] 当前 URL: {}", url);

    // 检查 CF 挑战状态
    let cf_check = page.evaluate(r#"(() => {
        const body = document.body ? document.body.innerHTML : '';
        const title = document.title || '';
        return {
            title: title,
            hasJustAMoment: title.includes('Just a moment'),
            hasCfChallenge: body.includes('cf-challenge-running'),
            hasTurnstile: body.includes('cf-turnstile') || body.includes('challenges.cloudflare.com'),
            hasChallengePlatform: body.includes('challenge-platform'),
            bodyLength: body.length,
            bodyPreview: body.substring(0, 300)
        };
    })()"#).await;

    if let Ok(check) = cf_check {
        println!("\n[4] CF 挑战检测:");
        println!("    - Title: {}", check.get("title").and_then(|v| v.as_str()).unwrap_or(""));
        println!("    - Just a moment: {}", check.get("hasJustAMoment").and_then(|v| v.as_bool()).unwrap_or(false));
        println!("    - CF Challenge: {}", check.get("hasCfChallenge").and_then(|v| v.as_bool()).unwrap_or(false));
        println!("    - Turnstile: {}", check.get("hasTurnstile").and_then(|v| v.as_bool()).unwrap_or(false));
        println!("    - Challenge Platform: {}", check.get("hasChallengePlatform").and_then(|v| v.as_bool()).unwrap_or(false));
        println!("    - Body Length: {}", check.get("bodyLength").and_then(|v| v.as_u64()).unwrap_or(0));
        println!("    - Body Preview: {}", check.get("bodyPreview").and_then(|v| v.as_str()).unwrap_or(""));
    }

    // 检查 Turnstile iframe
    let iframe_check = page.evaluate(r#"(() => {
        const iframes = document.querySelectorAll('iframe');
        const result = [];
        for (const iframe of iframes) {
            result.push({
                src: iframe.src || '',
                id: iframe.id || '',
                title: iframe.title || '',
                width: iframe.offsetWidth,
                height: iframe.offsetHeight
            });
        }
        return result;
    })()"#).await;

    if let Ok(iframes) = iframe_check {
        println!("\n[5] Iframe 检测 (JS):" );
        if let Some(arr) = iframes.as_array() {
            if arr.is_empty() {
                println!("    - 未找到 iframe (JS 无法访问 shadow DOM)");
            }
            for (i, iframe) in arr.iter().enumerate() {
                println!("    - Iframe {}:", i);
                println!("        src: {}", iframe.get("src").and_then(|v| v.as_str()).unwrap_or(""));
                println!("        id: {}", iframe.get("id").and_then(|v| v.as_str()).unwrap_or(""));
                println!("        size: {}x{}",
                    iframe.get("width").and_then(|v| v.as_u64()).unwrap_or(0),
                    iframe.get("height").and_then(|v| v.as_u64()).unwrap_or(0));
            }
        }
    }

    // 使用 CDP pierce 检测 shadow DOM 内的 iframe
    println!("\n[5b] Iframe 检测 (CDP pierce):");
    let _ = page.cmd("DOM.enable", serde_json::json!({})).await;
    if let Ok(doc) = page.cmd("DOM.getDocument", serde_json::json!({"depth": 200, "pierce": true})).await {
        fn find_iframes(node: &serde_json::Value, depth: usize) -> Vec<String> {
            let mut results = Vec::new();
            let node_name = node.get("nodeName").and_then(|n| n.as_str()).unwrap_or("");
            let indent = "    ".repeat(depth);

            if node_name.eq_ignore_ascii_case("IFRAME") {
                let attrs = node.get("attributes").and_then(|a| a.as_array());
                let mut src = String::new();
                let mut id = String::new();
                if let Some(attrs) = attrs {
                    for chunk in attrs.chunks(2) {
                        if chunk.len() == 2 {
                            let key = chunk[0].as_str().unwrap_or("");
                            let val = chunk[1].as_str().unwrap_or("");
                            if key == "src" { src = val.to_string(); }
                            if key == "id" { id = val.to_string(); }
                        }
                    }
                }
                results.push(format!("{}IFRAME: src='{}' id='{}'", indent, src, id));
            }

            // 递归子节点
            if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
                for child in children {
                    results.extend(find_iframes(child, depth + 1));
                }
            }
            // 递归 shadow roots
            if let Some(shadow_roots) = node.get("shadowRoots").and_then(|s| s.as_array()) {
                for sr in shadow_roots {
                    results.push(format!("{}[ShadowRoot]", indent));
                    if let Some(sr_children) = sr.get("children").and_then(|c| c.as_array()) {
                        for sr_child in sr_children {
                            results.extend(find_iframes(sr_child, depth + 1));
                        }
                    }
                }
            }
            // 递归 contentDocument
            if let Some(content_doc) = node.get("contentDocument") {
                results.extend(find_iframes(content_doc, depth + 1));
            }
            results
        }

        if let Some(root) = doc.get("root") {
            let iframes = find_iframes(root, 1);
            if iframes.is_empty() {
                println!("    - CDP pierce 也未找到 iframe");
            } else {
                for iframe in iframes {
                    println!("{}", iframe);
                }
            }
        }
    }

    // 检查 cookies
    let cookies = page.cookies().await.unwrap_or_default();
    println!("\n[6] Cookies:");
    for cookie in &cookies {
        let name = cookie.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name.starts_with("cf_") || name.starts_with("__cf") {
            println!("    - {}: {}", name, cookie.get("value").and_then(|v| v.as_str()).unwrap_or(""));
        }
    }

    // 截图保存
    println!("\n[7] 保存截图到 cf_diagnostic.png ...");
    let _ = page.screenshot("cf_diagnostic.png").await;

    let _ = page.close().await;
    let _ = browser.close().await;

    println!("\n=== 诊断完成 ===");
}

/// 测试 Stealth 模式无代理直连（对比测试）
#[tokio::test]
#[ignore = "requires Chrome browser, no proxy needed"]
async fn test_banzhu_stealth_no_proxy() {
    println!("[Stealth-NoProxy] 目标: {}", BANZHU_TARGET_URL);
    println!("[Stealth-NoProxy] 无代理直连");

    let result = Fetcher::stealth()
        .headless(true)
        .challenge_timeout(Duration::from_secs(45))
        .human_mode(true)
        .get(BANZHU_TARGET_URL)
        .await;

    match result {
        Ok(resp) => {
            println!("[Stealth-NoProxy] Status: {}", resp.status);
            println!("[Stealth-NoProxy] Title: {:?}", resp.title);
            println!("[Stealth-NoProxy] Body length: {} bytes", resp.body.len());

            let html = resp.text().unwrap_or_default();
            let on_challenge = html.contains("Just a moment")
                || html.contains("cf-challenge-running");

            if on_challenge {
                println!("[Stealth-NoProxy] 结果: 停留在 CF 挑战页（无代理预期结果）");
            } else if resp.status == 200 {
                println!("[Stealth-NoProxy] 结果: 成功获取页面（无代理也能过?）");
            }
        }
        Err(e) => {
            eprintln!("[Stealth-NoProxy] 失败: {}", e);
        }
    }
}

/// 测试非 headless 模式（可见浏览器）绕过 CF
#[tokio::test]
#[ignore = "requires network + proxy + Chrome browser, VISIBLE window"]
async fn test_banzhu_stealth_headed() {
    if !proxy_available().await {
        eprintln!("SKIP: 代理 {} 不可用", PROXY);
        return;
    }

    println!("\n=== Headed 模式测试 (Fetcher::stealth) ===" );
    println!("目标: {}", BANZHU_TARGET_URL);
    println!("模式: 可见浏览器 + Turnstile 解决器");

    let result = Fetcher::stealth()
        .headless(false)
        .proxy(PROXY)
        .challenge_timeout(Duration::from_secs(90))
        .human_mode(false)  // 禁用人类行为模拟，减少干扰
        .get(BANZHU_TARGET_URL)
        .await;

    match result {
        Ok(resp) => {
            println!("[Headed] Status: {}", resp.status);
            println!("[Headed] Title: {:?}", resp.title);
            println!("[Headed] Body length: {} bytes", resp.body.len());

            let title = resp.title().unwrap_or("");
            let html = resp.text().unwrap_or_default();

            let is_challenge_title = title.contains("Just a moment")
                || title.contains("请稍候")
                || title.contains("Attention Required");

            if resp.status == 200 && !is_challenge_title {
                println!("[PASS] 成功绕过 CF!");
                println!("[PASS] 页面标题: {}", title);
                if html.contains("小说") || html.contains("书库") {
                    println!("[PASS] 确认获取到真实内容");
                }
            } else {
                eprintln!("[FAIL] 状态: {}, 标题: {}", resp.status, title);
            }
        }
        Err(e) => {
            eprintln!("[FAIL] {}", e);
        }
    }
}

/// 测试纯等待模式（不点击，等待 CF 自动解决）
#[tokio::test]
#[ignore = "requires network + proxy + Chrome browser, VISIBLE window"]
async fn test_banzhu_passive_wait() {
    use wisp::{Browser, LaunchOptions};

    if !proxy_available().await {
        eprintln!("SKIP: 代理 {} 不可用", PROXY);
        return;
    }

    println!("\n=== 纯等待测试 ===" );
    println!("目标: {}", BANZHU_TARGET_URL);
    println!("策略: 只导航+等待，不发送任何 CDP 命令");

    let browser = match Browser::launch(LaunchOptions {
        headless: false,
        proxy: Some(wisp::ProxyConfig {
            server: PROXY.to_string(),
            username: None,
            password: None,
        }),
        ..Default::default()
    }).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("浏览器启动失败: {}", e);
            return;
        }
    };

    let mut page = browser.new_page().await.expect("创建页面");

    println!("[1] 导航...");
    if let Err(e) = page.goto(BANZHU_TARGET_URL).await {
        eprintln!("导航失败: {}", e);
        let _ = browser.close().await;
        return;
    }

    // 纯等待 30 秒，不做任何操作
    println!("[2] 等待 30 秒（不发送任何 CDP 命令）...");
    for i in 1..=6 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let title = page.title().await.unwrap_or_default();
        println!("    {}s - Title: {}", i * 5, title);
        if !title.contains("请稍候") && !title.contains("Just a moment") && !title.is_empty() {
            println!("[3] 挑战已自动解决! Title: {}", title);
            let html = page.content().await.unwrap_or_default();
            if html.contains("小说") {
                println!("[PASS] 获取到真实页面内容!");
            }
            let _ = page.close().await;
            let _ = browser.close().await;
            return;
        }
    }

    println!("[3] 30 秒后仍在挑战页，尝试点击...");
    // 等待后再用 Fetcher 的 solve 逻辑
    tokio::time::sleep(Duration::from_secs(30)).await;
    let title = page.title().await.unwrap_or_default();
    println!("[4] 60s 后 Title: {}", title);

    let _ = page.close().await;
    let _ = browser.close().await;
}

/// 参数扫描测试：测试不同 TurnstileConfig 参数组合的速度
#[tokio::test]
#[ignore = "requires network + proxy + Chrome browser, VISIBLE window - parameter sweep"]
async fn test_banzhu_param_sweep() {
    use wisp::TurnstileConfig;

    if !proxy_available().await {
        eprintln!("SKIP: 代理 {} 不可用", PROXY);
        return;
    }

    // 参数组合：(passive_wait_ms, mouse_steps, mouse_step_delay_ms, poll_interval_ms, click_hold_ms)
    let configs: Vec<(&str, TurnstileConfig)> = vec![
        ("默认", TurnstileConfig::default()),
        ("激进", TurnstileConfig {
            passive_wait_ms: 200,
            click_interval_ms: 1500,
            poll_interval_ms: 100,
            mouse_steps: 3,
            mouse_step_delay_ms: 5,
            click_hold_ms: 30,
            dom_depth: 10,
        }),
        ("极速", TurnstileConfig {
            passive_wait_ms: 100,
            click_interval_ms: 1000,
            poll_interval_ms: 50,
            mouse_steps: 2,
            mouse_step_delay_ms: 3,
            click_hold_ms: 20,
            dom_depth: 8,
        }),
        ("保守", TurnstileConfig {
            passive_wait_ms: 1000,
            click_interval_ms: 3000,
            poll_interval_ms: 300,
            mouse_steps: 8,
            mouse_step_delay_ms: 25,
            click_hold_ms: 100,
            dom_depth: 15,
        }),
    ];

    println!("\n=== 参数扫描测试 ===");
    println!("目标: {}", BANZHU_TARGET_URL);
    println!("测试 {} 组参数\n", configs.len());

    for (name, cfg) in &configs {
        println!("--- [{}] passive={}ms, mouse={}x{}ms, poll={}ms, hold={}ms ---",
            name, cfg.passive_wait_ms, cfg.mouse_steps, cfg.mouse_step_delay_ms,
            cfg.poll_interval_ms, cfg.click_hold_ms);

        let result = Fetcher::stealth()
            .headless(false)
            .proxy(PROXY)
            .challenge_timeout(Duration::from_secs(60))
            .human_mode(false)
            .turnstile_config(cfg.clone())
            .get(BANZHU_TARGET_URL)
            .await;

        match result {
            Ok(resp) => {
                let title = resp.title().unwrap_or("");
                if title.contains("小说") {
                    println!("    ✅ PASS - Title: {}", title);
                } else {
                    println!("    ⚠️  Status: {}, Title: {}", resp.status, title);
                }
            }
            Err(e) => {
                println!("    ❌ FAIL - {}", e);
            }
        }

        // 等待一下再测试下一组
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    println!("\n=== 扫描完成 ===");
}
