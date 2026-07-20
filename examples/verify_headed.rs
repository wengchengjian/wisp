use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-headed-{}", std::process::id()));
    println!("Launching HEADED browser...");
    let browser = Browser::launch(LaunchOptions {
        headless: false,  // HEADED mode!
        user_data_dir: Some(user_data.clone()),
        no_viewport: true,
        ..Default::default()
    }).await?;
    let page = browser.new_page().await?;
    println!("Testing Browserscan (headed)...");
    page.goto("https://www.browserscan.net/bot-detection").await?;
    tokio::time::sleep(Duration::from_secs(10)).await;
    page.screenshot("bs_headed.png").await?;
    println!("Screenshot: bs_headed.png");
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
