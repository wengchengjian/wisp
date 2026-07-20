use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-v3-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        user_data_dir: Some(user_data.clone()),
        ..Default::default()
    }).await?;
    let page = browser.new_page().await?;

    // Comprehensive main-world check
    let html = r#"data:text/html,<script>
        document.title = JSON.stringify({
            wd: typeof navigator.webdriver,
            plugins: navigator.plugins.length,
            chrome_app: typeof window.chrome !== 'undefined' && typeof window.chrome.app !== 'undefined',
            chrome_runtime: typeof window.chrome !== 'undefined' && typeof window.chrome.runtime !== 'undefined',
            notification: typeof Notification !== 'undefined' ? Notification.permission : 'N/A',
            outerH: window.outerHeight,
            innerH: window.innerHeight,
            screenW: screen.width,
            screenH: screen.height,
        });
    </script>"#;
    page.goto(html).await?;
    let title = page.evaluate_as_string("document.title").await?;
    println!("Stealth v3: {title}");

    // Sannysoft
    page.goto("https://bot.sannysoft.com/").await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    page.screenshot("verify_v3_sannysoft.png").await?;
    println!("Sannysoft done");

    // CreepJS
    page.goto("https://abrahamjuliot.github.io/creepjs/").await?;
    tokio::time::sleep(Duration::from_secs(12)).await;
    page.screenshot("verify_v3_creepjs.png").await?;
    println!("CreepJS done");

    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
