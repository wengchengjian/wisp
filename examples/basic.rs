//! Basic example: launch browser, navigate, evaluate JS, take screenshot.

use wisp::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Launching browser...");
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    }).await?;

    let page = browser.new_page().await?;

    // Navigate
    println!("Navigating to example.com...");
    page.goto("https://example.com").await?;

    // Evaluate JavaScript
    let title = page.evaluate_as_string("document.title").await?;
    println!("Page title: {title}");

    let webdriver = page.evaluate("typeof navigator.webdriver").await?;
    println!("typeof navigator.webdriver: {webdriver}");

    // Screenshot
    page.screenshot("example.png").await?;
    println!("Screenshot saved to example.png");

    browser.close().await?;
    println!("Done!");
    Ok(())
}
