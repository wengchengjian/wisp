use std::time::Duration;
use wisp::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let browser = Browser::launch(LaunchOptions { headless: false, ..Default::default() }).await?;
    let page = browser.new_page().await?;
    page.goto("https://www.bz555555555.com").await?;
    tokio::time::sleep(Duration::from_secs(10)).await;
    page.screenshot("cf_state.png").await?;
    println!("Screenshot saved: cf_state.png");
    browser.close().await?;
    Ok(())
}
