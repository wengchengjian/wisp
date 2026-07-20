use patchright_rs::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await?;

    let page = browser.new_page().await?;
    page.goto("https://example.com").await?;

    // Verify navigator.webdriver is null (not true)
    let webdriver = page.evaluate("navigator.webdriver").await?;
    println!("navigator.webdriver = {webdriver}");

    // Get page title
    let title = page.evaluate_as_string("document.title").await?;
    println!("Page title: {title}");

    // Screenshot
    page.screenshot("example.png").await?;
    println!("Screenshot saved to example.png");

    browser.close().await?;
    println!("Done! Browser closed.");

    Ok(())
}
