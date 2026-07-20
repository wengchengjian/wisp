use patchright_rs::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-full-{}", std::process::id()));
    println!("Launching browser via PIPE...");
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        user_data_dir: Some(user_data.clone()),
        ..Default::default()
    }).await?;
    println!("Browser launched!");

    let page = browser.new_page().await?;
    println!("Page created!");

    page.goto("https://example.com").await?;
    println!("Navigated to example.com");

    let title = page.evaluate_as_string("document.title").await?;
    println!("Page title: {title}");

    let wd = page.evaluate("typeof navigator.webdriver").await?;
    println!("typeof navigator.webdriver = {wd}");

    let sum = page.evaluate("1 + 2").await?;
    println!("1 + 2 = {sum}");

    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    println!("Done! Pipe-based CDP fully works!");
    Ok(())
}
