use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-native2-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions {
        headless: false,
        user_data_dir: Some(user_data.clone()),
        ..Default::default()
    }).await?;
    let page = browser.new_page().await?;
    // Navigate to a real page and check webdriver from main world
    page.goto("https://bot.sannysoft.com/").await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    // Read what the page's JS sees (via DOM)
    let html2 = r#"data:text/html,<script>
        const desc = Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver');
        document.title = JSON.stringify({
            native_value: navigator.webdriver,
            typeof_value: typeof navigator.webdriver,
            desc_exists: !!desc,
            desc_configurable: desc ? desc.configurable : null,
            desc_get_tostring: desc && desc.get ? desc.get.toString().substring(0, 60) : null,
        });
    </script>"#;
    page.goto(html2).await?;
    println!("Native check (headed): {}", page.evaluate_as_string("document.title").await?);
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
