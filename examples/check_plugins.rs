use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-final-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        user_data_dir: Some(user_data.clone()),
        ..Default::default()
    }).await?;
    let page = browser.new_page().await?;

    // Check what Sannysoft sees for plugins
    let html = r#"data:text/html,<script>
        const p = navigator.plugins;
        document.title = JSON.stringify({
            plugins_length: p.length,
            plugins_type: typeof p,
            plugins_tostring: Object.prototype.toString.call(p),
            plugin0: p.length > 0 ? p[0].name : 'none',
            plugin0_type: p.length > 0 ? typeof p[0] : 'none',
            item_method: typeof p.item,
            namedItem_method: typeof p.namedItem,
            webdriver: navigator.webdriver,
            typeof_webdriver: typeof navigator.webdriver,
        });
    </script>"#;
    page.goto(html).await?;
    let title = page.evaluate_as_string("document.title").await?;
    println!("Plugins check: {title}");

    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
