use std::time::Duration;
use wisp::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let browser = Browser::launch(LaunchOptions { headless: false, ..Default::default() }).await?;
    let page = browser.new_page().await?;

    // Inject banzhu-rs style FULL stealth (before navigation)
    let stealth = r#"
(function() {
    const o = (obj, prop, value) => Object.defineProperty(obj, prop, {
        get: () => value, enumerable: true, configurable: true
    });
    o(navigator, 'webdriver', false);
    o(navigator, 'plugins', [1,2,3,4,5]);
    o(navigator, 'languages', ['zh-CN','zh','en']);
    o(navigator, 'hardwareConcurrency', 8);
    o(navigator, 'deviceMemory', 8);
    o(navigator, 'platform', 'Win32');
    if (!window.chrome) { window.chrome = { runtime: {} }; }
    if (!navigator.connection) {
        o(navigator, 'connection', {
            downlink: 10, effectiveType: '4g', rtt: 50, saveData: false
        });
    }
    delete navigator.__proto__.webdriver;
})();
"#;
    page.cmd("Page.addScriptToEvaluateOnNewDocument", serde_json::json!({
        "source": stealth
    })).await?;

    page.goto("https://www.bz555555555.com").await?;
    println!("Navigated, waiting...");

    // Wait for JS challenge + try clicking Turnstile
    tokio::time::sleep(Duration::from_secs(6)).await;

    // Enable DOM for pierce
    page.cmd("DOM.enable", serde_json::json!({})).await?;

    for round in 1..=15 {
        // Check if passed
        let cookie = page.evaluate_as_string("document.cookie").await.unwrap_or_default();
        if cookie.contains("cf_clearance") {
            println!("[round {}] cf_clearance FOUND!", round);
            break;
        }
        let title = page.evaluate_as_string("document.title").await.unwrap_or_default();
        if !title.contains("Just a moment") && !title.is_empty() && round > 3 {
            let body_len = page.evaluate("document.body.innerHTML.length").await
                .map(|v| v.as_u64().unwrap_or(0)).unwrap_or(0);
            if body_len > 1000 {
                println!("[round {}] Content loaded! title='{}' len={}", round, title, body_len);
                break;
            }
        }

        // Try CDP click on Turnstile
        let doc = page.cmd("DOM.getDocument", serde_json::json!({"depth": 200, "pierce": true})).await;
        if let Ok(doc) = doc {
            if let Some(root) = doc.get("root") {
                if let Some(node_id) = find_turnstile(root) {
                    if let Ok(quads) = page.cmd("DOM.getContentQuads", serde_json::json!({"nodeId": node_id})).await {
                        if let Some(quad) = quads.get("quads").and_then(|q| q.as_array()).and_then(|q| q.first()).and_then(|q| q.as_array()) {
                            if quad.len() >= 8 {
                                let x = quad[0].as_f64().unwrap_or(0.0);
                                let y = quad[1].as_f64().unwrap_or(0.0);
                                let h = quad[5].as_f64().unwrap_or(65.0) - y;
                                let cx = x + 32.0 + ((round as f64 % 5.0) - 2.0) * 3.0;
                                let cy = y + h / 2.0 + ((round as f64 % 3.0) - 1.0) * 2.0;

                                // Ease-out mouse move
                                let sx = cx - 50.0 + ((round as f64 % 7.0) - 3.0) * 15.0;
                                let sy = cy - 40.0 + ((round as f64 % 5.0) - 2.0) * 12.0;
                                for i in 0..=10 {
                                    let t = i as f64 / 10.0;
                                    let ease = 1.0 - (1.0 - t) * (1.0 - t);
                                    let _ = page.cmd("Input.dispatchMouseEvent", serde_json::json!({
                                        "type": "mouseMoved",
                                        "x": sx + (cx - sx) * ease,
                                        "y": sy + (cy - sy) * ease
                                    })).await;
                                    tokio::time::sleep(Duration::from_millis(10 + (i as u64 * 5).min(40))).await;
                                }
                                // Click
                                let _ = page.cmd("Input.dispatchMouseEvent", serde_json::json!({
                                    "type": "mousePressed", "x": cx, "y": cy, "button": "left", "clickCount": 1
                                })).await;
                                tokio::time::sleep(Duration::from_millis(60 + (round as u64 * 13) % 50)).await;
                                let _ = page.cmd("Input.dispatchMouseEvent", serde_json::json!({
                                    "type": "mouseReleased", "x": cx, "y": cy, "button": "left", "clickCount": 1
                                })).await;
                                println!("[round {}] Clicked ({:.0},{:.0})", round, cx, cy);
                            }
                        }
                    }
                } else {
                    if round <= 3 { println!("[round {}] iframe not found", round); }
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    // Final check
    let title = page.evaluate_as_string("document.title").await?;
    let cookie = page.evaluate_as_string("document.cookie").await?;
    println!("\n=== FINAL: title='{}' cf_clearance={} ===", title, cookie.contains("cf_clearance"));

    browser.close().await?;
    Ok(())
}

fn find_turnstile(node: &serde_json::Value) -> Option<u32> {
    let name = node.get("nodeName").and_then(|n| n.as_str()).unwrap_or("");
    if name.eq_ignore_ascii_case("IFRAME") {
        if let Some(attrs) = node.get("attributes").and_then(|a| a.as_array()) {
            for pair in attrs.chunks(2) {
                if pair.len() == 2 {
                    let k = pair[0].as_str().unwrap_or("");
                    let v = pair[1].as_str().unwrap_or("");
                    if (k == "src" && v.contains("challenges.cloudflare.com")) ||
                       (k == "id" && v.contains("cf-chl-widget")) {
                        return node.get("nodeId").and_then(|i| i.as_u64()).map(|i| i as u32);
                    }
                }
            }
        }
    }
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for child in children { if let Some(r) = find_turnstile(child) { return Some(r); } }
    }
    if let Some(shadows) = node.get("shadowRoots").and_then(|s| s.as_array()) {
        for sr in shadows {
            if let Some(children) = sr.get("children").and_then(|c| c.as_array()) {
                for child in children { if let Some(r) = find_turnstile(child) { return Some(r); } }
            }
        }
    }
    if let Some(doc) = node.get("contentDocument") {
        if let Some(r) = find_turnstile(doc) { return Some(r); }
    }
    None
}
