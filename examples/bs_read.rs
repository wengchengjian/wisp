use patchright_rs::{Browser, LaunchOptions};
use std::time::Duration;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-bs3-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions { headless: false, user_data_dir: Some(user_data.clone()), no_viewport: true, ..Default::default() }).await?;
    let page = browser.new_page().await?;
    page.goto("https://www.browserscan.net/bot-detection").await?;
    tokio::time::sleep(Duration::from_secs(10)).await;
    // Try to read the detection result from the page
    let result = page.evaluate(r#"
        (() => {
            // Look for the verdict text on the page
            const body = document.body.innerText;
            const hasRobot = body.includes('Robot');
            const hasNormal = body.includes('Normal');
            const hasBot = body.includes('Bot');
            // Look for specific result elements
            const resultEls = document.querySelectorAll('[class*=result], [class*=score], [class*=verdict]');
            const resultTexts = Array.from(resultEls).map(e => e.textContent.trim()).filter(t => t.length < 100);
            return JSON.stringify({ hasRobot, hasNormal, hasBot, resultTexts: resultTexts.slice(0, 5), title: document.title });
        })()
    "#).await?;
    println!("Browserscan result: {result}");
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
