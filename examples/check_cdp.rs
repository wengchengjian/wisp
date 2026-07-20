use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-pipe-{}", std::process::id()));
    // Try with remote-debugging-pipe instead of port (less detectable)
    let browser = Browser::launch(LaunchOptions {
        headless: false,
        user_data_dir: Some(user_data.clone()),
        args: vec!["--disable-features=AutomationControllerForServiceWorker".to_string()],
        ..Default::default()
    }).await?;
    let page = browser.new_page().await?;
    
    // Check what's detectable
    let html = r#"data:text/html,<script>
        document.title = JSON.stringify({
            wd: typeof navigator.webdriver,
            wd_val: navigator.webdriver,
            // Check for CDP artifacts
            has_cdc: typeof document.querySelector('[cdc]') !== 'undefined',
            // Check performance entries for websocket
            perf_entries: performance.getEntriesByType('resource').filter(e => e.name.includes('ws://')).length,
        });
    </script>"#;
    page.goto(html).await?;
    println!("CDP artifacts: {}", page.evaluate_as_string("document.title").await?);
    
    // Check DevToolsActivePort file
    let dt_port = user_data.join("DevToolsActivePort");
    println!("DevToolsActivePort exists: {}", dt_port.exists());
    
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
