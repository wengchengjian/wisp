use std::time::Duration;
use wisp::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let browser = Browser::launch(LaunchOptions { headless: false, ..Default::default() }).await?;
    let page = browser.new_page().await?;
    page.goto("https://www.bz555555555.com").await?;

    // Wait for challenge to load
    tokio::time::sleep(Duration::from_secs(8)).await;

    // Enable DOM domain first
    page.cmd("DOM.enable", serde_json::json!({})).await?;

    // Use CDP DOM.getFlattenedDocument with pierce=true to see through shadow roots
    let dom = page.cmd("DOM.getFlattenedDocument", serde_json::json!({
        "depth": -1,
        "pierce": true
    })).await?;

    let nodes = dom.get("nodes").and_then(|n| n.as_array());
    if let Some(nodes) = nodes {
        println!("Total DOM nodes (pierced): {}", nodes.len());

        // Find all iframes
        let iframes: Vec<_> = nodes.iter().filter(|n| {
            n.get("nodeName").and_then(|n| n.as_str()) == Some("IFRAME")
        }).collect();
        println!("IFRAMEs found (pierced): {}", iframes.len());
        for iframe in &iframes {
            let attrs = iframe.get("attributes").and_then(|a| a.as_array());
            if let Some(attrs) = attrs {
                let attrs_str: Vec<String> = attrs.iter()
                    .filter_map(|a| a.as_str().map(|s| s.to_string()))
                    .collect();
                println!("  IFRAME attrs: {:?}", attrs_str);
            }
        }

        // Find shadow roots
        let shadows: Vec<_> = nodes.iter().filter(|n| {
            n.get("shadowRoots").is_some()
        }).collect();
        println!("Nodes with shadow roots: {}", shadows.len());
        for s in &shadows {
            let name = s.get("nodeName").and_then(|n| n.as_str()).unwrap_or("?");
            let roots = s.get("shadowRoots").and_then(|r| r.as_array());
            if let Some(roots) = roots {
                for root in roots {
                    let mode = root.get("shadowRootType").and_then(|m| m.as_str()).unwrap_or("?");
                    println!("  Shadow root on <{}> mode={}", name, mode);
                }
            }
        }

        // Find turnstile-related elements
        let cf_nodes: Vec<_> = nodes.iter().filter(|n| {
            let attrs = n.get("attributes").and_then(|a| a.as_array());
            if let Some(attrs) = attrs {
                attrs.iter().any(|a| {
                    a.as_str().map_or(false, |s| s.contains("turnstile") || s.contains("cf-chl") || s.contains("challenge"))
                })
            } else { false }
        }).collect();
        println!("CF-related nodes: {}", cf_nodes.len());
        for n in &cf_nodes {
            let name = n.get("nodeName").and_then(|n| n.as_str()).unwrap_or("?");
            let attrs = n.get("attributes").and_then(|a| a.as_array());
            let attrs_str: Vec<String> = attrs.map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
            println!("  <{}> {:?}", name, &attrs_str[..attrs_str.len().min(6)]);
        }
    }

    // Also check: is our shadow_dom patch working?
    let patch_check = page.evaluate(r#"(() => {
        const test = document.createElement('div');
        const shadow = test.attachShadow({mode: 'closed'});
        return { mode: shadow.mode, patched: shadow.mode === 'open' };
    })()"#).await?;
    println!("\nShadow DOM patch check: {}", patch_check);

    browser.close().await?;
    Ok(())
}
