use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ud = std::env::temp_dir().join(format!("pr-h4-{}", std::process::id()));
    println!("HEADED test (no --no-sandbox)...");
    let browser = Browser::launch(LaunchOptions { headless: false, user_data_dir: Some(ud.clone()), no_viewport: true, ..Default::default() }).await?;
    println!("Launched!");
    let page = browser.new_page().await?;
    println!("Page!");
    page.goto("https://www.browserscan.net/bot-detection").await?;
    println!("Navigated!");
    tokio::time::sleep(Duration::from_secs(10)).await;
    page.screenshot("bs_headed_final.png").await?;
    println!("Done! Screenshot saved.");
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&ud);
    Ok(())
}
