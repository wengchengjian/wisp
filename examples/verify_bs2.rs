use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-bs-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions { headless: true, user_data_dir: Some(user_data.clone()), ..Default::default() }).await?;
    let page = browser.new_page().await?;

    // Check main world webdriver via DOM
    page.goto("data:text/html,<script>document.title=JSON.stringify({wd:typeof navigator.webdriver,wd_val:navigator.webdriver,chrome:typeof window.chrome!=='undefined',plugins:navigator.plugins.length})</script>").await?;
    let title = page.evaluate_as_string("document.title").await?;
    println!("Main world: {title}");

    // Browserscan
    println!("Testing Browserscan...");
    page.goto("https://www.browserscan.net/bot-detection").await?;
    tokio::time::sleep(Duration::from_secs(10)).await;
    page.screenshot("bs_result.png").await?;
    println!("Screenshot saved: bs_result.png");

    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
