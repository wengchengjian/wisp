use std::path::PathBuf;

pub(super) async fn run_open(
    url: String,
    headless: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use wisp::{Browser, LaunchOptions};
    println!("Opening {url}...");
    let browser = Browser::launch(LaunchOptions {
        headless,
        ..Default::default()
    })
    .await?;
    let mut page = browser.new_page().await?;
    page.goto(&url).await?;
    println!("Page loaded. Press Ctrl+C to close.");
    tokio::signal::ctrl_c().await?;
    browser.close().await?;
    Ok(())
}

pub(super) async fn run_screenshot(
    url: String,
    output: PathBuf,
    headed: bool,
    wait: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    use wisp::{Browser, LaunchOptions};
    println!("Screenshot: {url}");
    let browser = Browser::launch(LaunchOptions {
        headless: !headed,
        ..Default::default()
    })
    .await?;
    let mut page = browser.new_page().await?;
    page.goto(&url).await?;
    if wait > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
    }
    page.screenshot(output.to_str().unwrap_or("screenshot.png"))
        .await?;
    println!("Saved: {}", output.display());
    browser.close().await?;
    Ok(())
}

pub(super) async fn run_eval(
    expression: String,
    url: String,
    headless: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use wisp::{Browser, LaunchOptions};
    let browser = Browser::launch(LaunchOptions {
        headless,
        ..Default::default()
    })
    .await?;
    let mut page = browser.new_page().await?;
    if url != "about:blank" {
        page.goto(&url).await?;
    }
    let result = page.evaluate(&expression).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    browser.close().await?;
    Ok(())
}

pub(super) async fn run_dump(
    url: String,
    headless: bool,
    wait: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    use wisp::{Browser, LaunchOptions};
    let browser = Browser::launch(LaunchOptions {
        headless,
        ..Default::default()
    })
    .await?;
    let mut page = browser.new_page().await?;
    page.goto(&url).await?;
    if wait > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
    }
    let text = page.evaluate_as_string("document.body.innerText").await?;
    println!("{text}");
    browser.close().await?;
    Ok(())
}
