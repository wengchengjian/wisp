use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-pipe-{}", std::process::id()));
    println!("Launching browser via PIPE...");
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        user_data_dir: Some(user_data.clone()),
        ..Default::default()
    }).await?;
    println!("Browser launched!");

    let page = browser.new_page().await?;
    println!("Page created!");

    page.goto("about:blank").await?;
    println!("Navigated to about:blank");

    let result = page.evaluate("1 + 2").await?;
    println!("evaluate('1 + 2') = {result}");

    let wd = page.evaluate("typeof navigator.webdriver").await?;
    println!("typeof navigator.webdriver = {wd}");

    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    println!("Done! Pipe-based CDP works!");
    Ok(())
}
