use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-final2-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions { headless: true, user_data_dir: Some(user_data.clone()), ..Default::default() }).await?;
    let page = browser.new_page().await?;

    // Sannysoft
    println!("Testing Sannysoft...");
    page.goto("https://bot.sannysoft.com/").await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    page.screenshot("final_sannysoft.png").await?;

    // Browserscan
    println!("Testing Browserscan...");
    page.goto("https://www.browserscan.net/bot-detection").await?;
    tokio::time::sleep(Duration::from_secs(8)).await;
    page.screenshot("final_browserscan.png").await?;

    // CreepJS
    println!("Testing CreepJS...");
    page.goto("https://abrahamjuliot.github.io/creepjs/").await?;
    tokio::time::sleep(Duration::from_secs(12)).await;
    page.screenshot("final_creepjs.png").await?;

    println!("All done!");
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
