use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-bs2-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions { headless: false, user_data_dir: Some(user_data.clone()), no_viewport: true, ..Default::default() }).await?;
    let page = browser.new_page().await?;
    page.goto("https://www.browserscan.net/bot-detection").await?;
    tokio::time::sleep(Duration::from_secs(10)).await;
    page.screenshot("final3_browserscan.png").await?;
    println!("Done");
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
