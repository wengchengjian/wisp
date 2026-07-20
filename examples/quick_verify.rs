use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-quick-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        user_data_dir: Some(user_data.clone()),
        ..Default::default()
    }).await?;
    let page = browser.new_page().await?;

    // Quick main-world webdriver check
    let html = r#"data:text/html,<script>document.title = JSON.stringify({wd: navigator.webdriver, typeof_wd: typeof navigator.webdriver, plugins: navigator.plugins.length, proto_wd: Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver') ? 'exists' : 'missing'})</script>"#;
    page.goto(html).await?;
    let title = page.evaluate_as_string("document.title").await?;
    println!("Quick check: {title}");

    // Sannysoft
    page.goto("https://bot.sannysoft.com/").await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    page.screenshot("verify_sannysoft2.png").await?;
    println!("Sannysoft screenshot saved");

    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
