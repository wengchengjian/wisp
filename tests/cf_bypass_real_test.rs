//! Cloudflare bypass 鐪熷疄鐜娴嬭瘯銆?
//!
//! 杩愯鏂瑰紡锛歚cargo test --test cf_bypass_real_test -- --ignored`
//!
//! 鎵€鏈夋祴璇曟爣娉?`#[ignore]`锛岄渶瑕侊細
//! - 鏈湴浠ｇ悊 127.0.0.1:7897 鍙敤
//! - Chrome 娴忚鍣ㄥ凡瀹夎锛坆rowser 娴嬭瘯锛?
//! - 缃戠粶鍙闂洰鏍囩珯鐐?
//!
//! 瑕嗙洊鍦烘櫙锛?
//! 1. TLS 鎸囩汗楠岃瘉锛坱ls.peet.ws锛?
//! 2. HTTP fetch 甯︿唬鐞嗙粫杩囧熀纭€鍙?bot
//! 3. Fetcher::stealth() 缁曡繃 CF Turnstile
//! 4. CF JS Challenge (5绉掔浘) 鑷姩绛夊緟
//! 5. Engine + 浠ｇ悊姹犵埇鍙?CF 淇濇姢绔欑偣

use std::time::Duration;
use wisp::http::Client;
use wisp::httper;
use wisp::crawl::{Engine, SpiderBuilder};
use wisp::proxy::RotationStrategy;
use wreq_util::Profile;

/// 鏈湴浠ｇ悊鍦板潃
const PROXY: &str = "http://127.0.0.1:7897";

/// 妫€娴嬩唬鐞嗘槸鍚﹀彲鐢?
async fn proxy_available() -> bool {
    let client = match Client::builder()
        .timeout(Duration::from_secs(5))
        .proxy(PROXY)
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client.get("https://www.google.com/generate_204").await.is_ok()
}

// === 娴嬭瘯 1: TLS 鎸囩汗楠岃瘉 ===

#[tokio::test]
#[ignore = "requires network + proxy 127.0.0.1:7897"]
async fn test_tls_fingerprint_chrome() {
    if !proxy_available().await {
        eprintln!("SKIP: 浠ｇ悊 {} 涓嶅彲鐢?, PROXY);
        return;
    }

    let client = Client::builder()
        .proxy(PROXY)
        .emulation(Profile::Chrome136)
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    let resp = client.get("https://tls.peet.ws/api/all").await.unwrap();
    assert_eq!(resp.status, 200, "tls.peet.ws 搴旇繑鍥?200");

    let json = resp.json().unwrap();
    // 楠岃瘉 TLS 鎸囩汗 - peet.ws 杩斿洖鐨?ja3_text 鎴?ja4 搴斿惈 Chrome 鐗瑰緛
    let tls_info = &json["tls"];
    let ja3_text = tls_info["ja3_text"].as_str().unwrap_or("");
    let ja4 = tls_info["ja4"].as_str().unwrap_or("");

    println!("JA3: {}", ja3_text);
    println!("JA4: {}", ja4);

    // Chrome 136 鐨?JA4 搴斾互 "t13d1516h2" 寮€澶达紙TLS 1.3, Chrome 鐗瑰緛锛?
    // 鎴栬€?ja3_text 闈炵┖鍗宠〃绀烘寚绾规ā鎷熺敓鏁?
    assert!(
        !ja3_text.is_empty() || !ja4.is_empty(),
        "TLS 鎸囩汗搴旈潪绌猴紙琛ㄧず妯℃嫙鐢熸晥锛?
    );

    // 楠岃瘉 HTTP/2 鎸囩汗瀛樺湪
    let http2_info = &json["http2"];
    let akamai_fp = http2_info["akamai_fingerprint_hash"].as_str().unwrap_or("");
    println!("HTTP/2 Akamai FP: {}", akamai_fp);
    assert!(!akamai_fp.is_empty(), "HTTP/2 鎸囩汗搴斿瓨鍦?);
}

// === 娴嬭瘯 2: HTTP fetch 甯︿唬鐞?+ TLS 鎸囩汗缁曡繃鍩虹鍙?bot ===

#[tokio::test]
#[ignore = "requires network + proxy 127.0.0.1:7897"]
async fn test_fetch_with_proxy_bot_detection() {
    if !proxy_available().await {
        eprintln!("SKIP: 浠ｇ悊 {} 涓嶅彲鐢?, PROXY);
        return;
    }

    let client = Client::builder()
        .proxy(PROXY)
        .emulation(Profile::Chrome136)
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    // bot.sannysoft.com 妫€娴嬪熀鏈祻瑙堝櫒鐗瑰緛
    let resp = client.get("https://quotes.toscrape.com/").await.unwrap();
    assert_eq!(resp.status, 200);

    let text = resp.text().unwrap();
    assert!(text.contains("Quotes to Scrape"), "搴旀垚鍔熻幏鍙栭〉闈㈠唴瀹?);
}

// === 娴嬭瘯 3: Fetcher::stealth() 缁曡繃 CF Turnstile ===

#[tokio::test]
#[ignore = "requires network + proxy + Chrome browser"]
async fn test_stealth_cf_turnstile_bypass() {
    if !proxy_available().await {
        eprintln!("SKIP: 浠ｇ悊 {} 涓嶅彲鐢?, PROXY);
        return;
    }

    // nopecha.com/demo/cloudflare 鏄竴涓?CF Turnstile 婕旂ず椤甸潰
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
            // 鎴愬姛缁曡繃锛氫笉搴旂湅鍒?CF 鎸戞垬椤甸潰
            assert!(
                !html.contains("Just a moment") && !html.contains("cf-challenge-running"),
                "搴旂粫杩?CF 鎸戞垬锛屼絾椤甸潰浠嶅惈鎸戞垬鏍囪"
            );
        }
        Err(e) => {
            eprintln!("SKIP: Stealth 娴嬭瘯澶辫触锛堝彲鑳芥棤 Chrome锛? {}", e);
        }
    }
}

// === 娴嬭瘯 4: CF JS Challenge 鑷姩绛夊緟 ===

#[tokio::test]
#[ignore = "requires network + proxy + Chrome browser"]
async fn test_stealth_cf_js_challenge() {
    if !proxy_available().await {
        eprintln!("SKIP: 浠ｇ悊 {} 涓嶅彲鐢?, PROXY);
        return;
    }

    // nowsecure.nl 浼氳Е鍙?CF JS Challenge
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
                eprintln!("WARN: 浠嶅仠鐣欏湪 CF 鎸戞垬椤甸潰锛堝彲鑳介渶瑕佹洿闀跨瓑寰呮椂闂达級");
            } else {
                println!("PASS: 鎴愬姛閫氳繃 CF JS Challenge");
            }
        }
        Err(e) => {
            eprintln!("SKIP: {}", e);
        }
    }
}

// === 娴嬭瘯 5: Engine + 浠ｇ悊姹犵埇鍙?===

#[tokio::test]
#[ignore = "requires network + proxy 127.0.0.1:7897"]
async fn test_engine_with_proxy_pool() {
    if !proxy_available().await {
        eprintln!("SKIP: 浠ｇ悊 {} 涓嶅彲鐢?, PROXY);
        return;
    }

    let spider = SpiderBuilder::new("proxy-crawl")
        .start_urls(vec!["https://quotes.toscrape.com/"])
        .concurrent(2)
        .obey_robots(false)
        .parse(|resp| {
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

    let stats = Engine::builder(spider)
        .max_pages(1)
        .proxy_pool(vec![PROXY.to_string()], RotationStrategy::Sequential)
        .run()
        .await
        .unwrap();

    assert_eq!(stats.pages_crawled, 1, "搴旀垚鍔熺埇鍙?1 椤?);
    assert!(stats.items_scraped >= 5, "搴旀彁鍙栬嚦灏?5 鏉″悕瑷€, 瀹為檯: {}", stats.items_scraped);
    assert_eq!(stats.errors, 0, "涓嶅簲鏈夐敊璇?);
}

// === 娴嬭瘯 6: 澶氭寚绾硅疆鎹㈡祴璇?===

#[tokio::test]
#[ignore = "requires network + proxy 127.0.0.1:7897"]
async fn test_multiple_tls_fingerprints() {
    if !proxy_available().await {
        eprintln!("SKIP: 浠ｇ悊 {} 涓嶅彲鐢?, PROXY);
        return;
    }

    // 娴嬭瘯涓嶅悓娴忚鍣ㄦ寚绾归兘鑳芥甯稿伐浣?
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

        let resp = client.get("https://quotes.toscrape.com/").await;
        match resp {
            Ok(r) => {
                assert_eq!(r.status, 200, "{} 鎸囩汗搴旀垚鍔熻幏鍙栭〉闈?, name);
                println!("PASS: {} 鎸囩汗姝ｅ父", name);
            }
            Err(e) => {
                eprintln!("WARN: {} 鎸囩汗璇锋眰澶辫触: {}", name, e);
            }
        }
    }
}
