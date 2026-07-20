use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;

/// Visit real anti-bot detection sites and take screenshots for verification.
/// This tests against the same sites listed in patchright's stealth documentation.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("patchright-verify-{}", std::process::id()));

    println!("Launching browser...");
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        user_data_dir: Some(user_data.clone()),
        ..Default::default()
    })
    .await?;

    let page = browser.new_page().await?;

    // Test 1: Sannysoft (basic automation detection)
    println!("\n=== Test 1: Sannysoft ===");
    page.goto("https://bot.sannysoft.com/").await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    page.screenshot("verify_sannysoft.png").await?;
    println!("Screenshot saved: verify_sannysoft.png");

    // Test 2: Browserscan (comprehensive fingerprint detection)
    println!("\n=== Test 2: Browserscan ===");
    page.goto("https://www.browserscan.net/bot-detection").await?;
    tokio::time::sleep(Duration::from_secs(8)).await;
    page.screenshot("verify_browserscan.png").await?;
    println!("Screenshot saved: verify_browserscan.png");

    // Test 3: CreepJS (advanced fingerprint analysis)
    println!("\n=== Test 3: CreepJS ===");
    page.goto("https://abrahamjuliot.github.io/creepjs/").await?;
    tokio::time::sleep(Duration::from_secs(10)).await;
    page.screenshot("verify_creepjs.png").await?;
    println!("Screenshot saved: verify_creepjs.png");

    // Test 4: Fingerprint.com bot detection
    println!("\n=== Test 4: Fingerprint.com ===");
    page.goto("https://fingerprint.com/products/bot-detection/").await?;
    tokio::time::sleep(Duration::from_secs(8)).await;
    page.screenshot("verify_fingerprint.png").await?;
    println!("Screenshot saved: verify_fingerprint.png");

    // Test 5: Incolumitas (comprehensive bot detection)
    println!("\n=== Test 5: Incolumitas ===");
    page.goto("https://bot.incolumitas.com/").await?;
    tokio::time::sleep(Duration::from_secs(10)).await;
    page.screenshot("verify_incolumitas.png").await?;
    println!("Screenshot saved: verify_incolumitas.png");

    // Quick JS-based verification on each site
    println!("\n=== Quick JS Checks ===");
    page.goto("about:blank").await?;
    
    // Verify key stealth properties from main world
    let html = r#"data:text/html,<script>
        document.title = JSON.stringify({
            webdriver: typeof navigator.webdriver,
            webdriver_val: navigator.webdriver,
            plugins: navigator.plugins.length,
            languages: navigator.languages.length,
            chrome: typeof window.chrome !== 'undefined',
            chrome_runtime: typeof window.chrome !== 'undefined' && typeof window.chrome.runtime !== 'undefined',
            ua: navigator.userAgent.substring(0, 80),
            platform: navigator.platform,
            hardwareConcurrency: navigator.hardwareConcurrency,
            deviceMemory: navigator.deviceMemory,
            maxTouchPoints: navigator.maxTouchPoints,
        });
    </script>"#;
    page.goto(html).await?;
    let title = page.evaluate_as_string("document.title").await?;
    println!("Stealth properties: {}", serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(&title)?)?);

    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    println!("\nDone! Check the screenshots for detection results.");

    Ok(())
}
