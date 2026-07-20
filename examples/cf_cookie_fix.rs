use std::time::Duration;
use wisp::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let browser = Browser::launch(LaunchOptions {
        headless: false,
        args: vec!["--disable-gpu".into(), "--no-sandbox".into()],
        ..Default::default()
    }).await?;
    let page = browser.new_page().await?;
    let stealth = r#"(function(){const o=(obj,prop,value)=>Object.defineProperty(obj,prop,{get:()=>value,enumerable:true,configurable:true});o(navigator,'webdriver',false);o(navigator,'plugins',[1,2,3,4,5]);o(navigator,'languages',['zh-CN','zh','en']);o(navigator,'hardwareConcurrency',8);o(navigator,'deviceMemory',8);o(navigator,'platform','Win32');if(!window.chrome){window.chrome={runtime:{}};}if(!navigator.connection){o(navigator,'connection',{downlink:10,effectiveType:'4g',rtt:50,saveData:false});}delete navigator.__proto__.webdriver;})()"#;
    page.cmd("Page.addScriptToEvaluateOnNewDocument", serde_json::json!({"source": stealth})).await?;
    page.goto("https://www.bz555555555.com").await?;
    println!("Navigated");
    tokio::time::sleep(Duration::from_secs(5)).await;
    page.cmd("DOM.enable", serde_json::json!({})).await?;

    for round in 1..=20u32 {
        // Use CDP Network.getCookies (sees httpOnly cookies!)
        if let Ok(resp) = page.cmd("Network.getCookies", serde_json::json!({})).await {
            let has_cf = resp.pointer("/cookies").and_then(|c| c.as_array()).map(|arr| {
                arr.iter().any(|c| c.get("name").and_then(|n| n.as_str()) == Some("cf_clearance"))
            }).unwrap_or(false);
            if has_cf {
                println!("[round {}] cf_clearance FOUND via Network.getCookies!", round);
                tokio::time::sleep(Duration::from_secs(3)).await;
                let title = page.evaluate_as_string("document.title").await?;
                let body = page.evaluate_as_string("document.body.innerText").await.unwrap_or_default();
                println!("Title: {}", title);
                println!("Body: {}", &body[..body.len().min(300)]);
                browser.close().await?;
                return Ok(());
            }
        }

        // Click turnstile via CDP pierce
        if round > 1 {
            if let Ok(doc) = page.cmd("DOM.getDocument", serde_json::json!({"depth": 200, "pierce": true})).await {
                if let Some(root) = doc.get("root") {
                    if let Some(nid) = find_tf(root) {
                        if let Ok(q) = page.cmd("DOM.getContentQuads", serde_json::json!({"nodeId": nid})).await {
                            if let Some(quad) = q.pointer("/quads/0").and_then(|q| q.as_array()) {
                                if quad.len() >= 8 {
                                    let x = quad[0].as_f64().unwrap_or(0.0);
                                    let y = quad[1].as_f64().unwrap_or(0.0);
                                    let h = quad[5].as_f64().unwrap_or(65.0) - y;
                                    let cx = x + 32.0 + ((round as f64 % 5.0) - 2.0) * 3.0;
                                    let cy = y + h / 2.0 + ((round as f64 % 3.0) - 1.0) * 2.0;
                                    let sx = cx - 50.0 + ((round as f64 % 7.0) - 3.0) * 15.0;
                                    let sy = cy - 40.0 + ((round as f64 % 5.0) - 2.0) * 12.0;
                                    for i in 0..=10u64 {
                                        let t = i as f64 / 10.0;
                                        let e = 1.0 - (1.0 - t) * (1.0 - t);
                                        let _ = page.cmd("Input.dispatchMouseEvent", serde_json::json!({
                                            "type": "mouseMoved", "x": sx + (cx-sx)*e, "y": sy + (cy-sy)*e
                                        })).await;
                                        tokio::time::sleep(Duration::from_millis(10 + (i*5).min(40))).await;
                                    }
                                    let _ = page.cmd("Input.dispatchMouseEvent", serde_json::json!({
                                        "type": "mousePressed", "x": cx, "y": cy, "button": "left", "clickCount": 1
                                    })).await;
                                    tokio::time::sleep(Duration::from_millis(70)).await;
                                    let _ = page.cmd("Input.dispatchMouseEvent", serde_json::json!({
                                        "type": "mouseReleased", "x": cx, "y": cy, "button": "left", "clickCount": 1
                                    })).await;
                                    println!("[round {}] click ({:.0},{:.0})", round, cx, cy);
                                }
                            }
                        }
                    } else if round <= 3 {
                        println!("[round {}] no iframe", round);
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    println!("TIMEOUT - no cf_clearance");
    browser.close().await?;
    Ok(())
}

fn find_tf(n: &serde_json::Value) -> Option<u32> {
    let nm = n.get("nodeName").and_then(|n| n.as_str()).unwrap_or("");
    if nm.eq_ignore_ascii_case("IFRAME") {
        if let Some(a) = n.get("attributes").and_then(|a| a.as_array()) {
            for p in a.chunks(2) {
                if p.len() == 2 {
                    let k = p[0].as_str().unwrap_or("");
                    let v = p[1].as_str().unwrap_or("");
                    if (k == "src" && v.contains("challenges.cloudflare.com"))
                        || (k == "id" && v.contains("cf-chl-widget"))
                    {
                        return n.get("nodeId").and_then(|i| i.as_u64()).map(|i| i as u32);
                    }
                }
            }
        }
    }
    if let Some(c) = n.get("children").and_then(|c| c.as_array()) {
        for ch in c { if let Some(r) = find_tf(ch) { return Some(r); } }
    }
    if let Some(s) = n.get("shadowRoots").and_then(|s| s.as_array()) {
        for sr in s {
            if let Some(c) = sr.get("children").and_then(|c| c.as_array()) {
                for ch in c { if let Some(r) = find_tf(ch) { return Some(r); } }
            }
        }
    }
    if let Some(d) = n.get("contentDocument") {
        if let Some(r) = find_tf(d) { return Some(r); }
    }
    None
}
