use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-headed-{}", std::process::id()));
    println!("Launching in HEADED mode...");
    let browser = Browser::launch(LaunchOptions {
        headless: false,  // HEADED mode for full stealth
        user_data_dir: Some(user_data.clone()),
        no_viewport: true,
        ..Default::default()
    }).await?;
    let page = browser.new_page().await?;

    // Quick stealth check
    let html = r#"data:text/html,<script>document.title=JSON.stringify({wd:typeof navigator.webdriver,screenW:screen.width,screenH:screen.height,plugins:navigator.plugins.length,chrome:typeof window.chrome!=='undefined'})</script>"#;
    page.goto(html).await?;
    println!("Stealth: {}", page.evaluate_as_string("document.title").await?);

    // Browserscan
    println!("Testing Browserscan (headed)...");
    page.goto("https://www.browserscan.net/bot-detection").await?;
    tokio::time::sleep(Duration::from_secs(10)).await;
    page.screenshot("headed_browserscan.png").await?;
    println!("Browserscan screenshot saved");

    // Sannysoft
    println!("Testing Sannysoft (headed)...");
    page.goto("https://bot.sannysoft.com/").await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    page.screenshot("headed_sannysoft.png").await?;
    println!("Sannysoft screenshot saved");

    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    println!("Done!");
    Ok(())
}
