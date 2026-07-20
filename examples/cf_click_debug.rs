use std::time::Duration;
use wisp::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let browser = Browser::launch(LaunchOptions { headless: false, ..Default::default() }).await?;
    let page = browser.new_page().await?;
    page.goto("https://www.bz555555555.com").await?;
    tokio::time::sleep(Duration::from_secs(8)).await;

    // Enable DOM
    page.cmd("DOM.enable", serde_json::json!({})).await?;

    // Get pierced DOM
    let doc = page.cmd("DOM.getDocument", serde_json::json!({"depth": 200, "pierce": true})).await?;
    let root = doc.get("root").unwrap();

    // Find turnstile iframe
    fn find_iframe(node: &serde_json::Value) -> Option<(u32, String)> {
        let name = node.get("nodeName").and_then(|n| n.as_str()).unwrap_or("");
        let attrs = node.get("attributes").and_then(|a| a.as_array());
        if name.eq_ignore_ascii_case("IFRAME") {
            if let Some(attrs) = attrs {
                for pair in attrs.chunks(2) {
                    if pair.len() == 2 {
                        let k = pair[0].as_str().unwrap_or("");
                        let v = pair[1].as_str().unwrap_or("");
                        if (k == "src" && v.contains("challenges.cloudflare.com")) ||
                           (k == "id" && v.contains("cf-chl-widget")) {
                            let id = node.get("nodeId").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
                            return Some((id, v.to_string()));
                        }
                    }
                }
            }
        }
        if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
            for child in children {
                if let Some(r) = find_iframe(child) { return Some(r); }
            }
        }
        if let Some(shadows) = node.get("shadowRoots").and_then(|s| s.as_array()) {
            for sr in shadows {
                if let Some(children) = sr.get("children").and_then(|c| c.as_array()) {
                    for child in children {
                        if let Some(r) = find_iframe(child) { return Some(r); }
                    }
                }
            }
        }
        if let Some(doc) = node.get("contentDocument") {
            if let Some(r) = find_iframe(doc) { return Some(r); }
        }
        None
    }

    match find_iframe(root) {
        Some((node_id, src)) => {
            println!("FOUND iframe nodeId={} src={}", node_id, &src[..src.len().min(80)]);

            // Get coordinates
            let quads = page.cmd("DOM.getContentQuads", serde_json::json!({"nodeId": node_id})).await?;
            println!("Quads: {}", serde_json::to_string_pretty(&quads)?);

            if let Some(quad) = quads.get("quads").and_then(|q| q.as_array()).and_then(|q| q.first()).and_then(|q| q.as_array()) {
                if quad.len() >= 8 {
                    let x = quad[0].as_f64().unwrap_or(0.0);
                    let y = quad[1].as_f64().unwrap_or(0.0);
                    let h = quad[5].as_f64().unwrap_or(0.0) - y;
                    println!("iframe pos: x={:.0} y={:.0} h={:.0}", x, y, h);
                    println!("checkbox target: ({:.0}, {:.0})", x + 32.0, y + h / 2.0);

                    // Try clicking
                    let cx = x + 32.0;
                    let cy = y + h / 2.0;

                    // Move mouse
                    for i in 0..=10 {
                        let t = i as f64 / 10.0;
                        let ease = 1.0 - (1.0 - t) * (1.0 - t);
                        let mx = (cx - 50.0) + 50.0 * ease;
                        let my = (cy - 40.0) + 40.0 * ease;
                        let _ = page.cmd("Input.dispatchMouseEvent", serde_json::json!({
                            "type": "mouseMoved", "x": mx, "y": my
                        })).await;
                        tokio::time::sleep(Duration::from_millis(15)).await;
                    }

                    tokio::time::sleep(Duration::from_millis(100)).await;

                    // Click
                    let _ = page.cmd("Input.dispatchMouseEvent", serde_json::json!({
                        "type": "mousePressed", "x": cx, "y": cy, "button": "left", "clickCount": 1
                    })).await;
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    let _ = page.cmd("Input.dispatchMouseEvent", serde_json::json!({
                        "type": "mouseReleased", "x": cx, "y": cy, "button": "left", "clickCount": 1
                    })).await;
                    println!("CLICK dispatched at ({:.0}, {:.0})", cx, cy);

                    // Wait and check
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    let cookie = page.evaluate_as_string("document.cookie").await?;
                    let title = page.evaluate_as_string("document.title").await?;
                    println!("After click: title='{}' cookie_has_cf={}", title, cookie.contains("cf_clearance"));
                }
            }
        }
        None => println!("iframe NOT FOUND in pierced DOM"),
    }

    browser.close().await?;
    Ok(())
}
