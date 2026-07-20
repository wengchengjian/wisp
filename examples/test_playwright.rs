use patchright_rs::driver::Driver;
use base64::Engine;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("patchright_rs=debug,info")
        .init();

    println!("=== Patchright Playwright Protocol Test ===\n");

    // 1. Launch the driver
    println!("[1] Launching driver...");
    let mut driver = Driver::launch().await?;
    println!("    Driver connected!\n");

    // 2. Initialize the connection
    println!("[2] Initializing Playwright protocol...");
    let pw_guid = driver.initialize().await?;
    println!("    Playwright guid: {}\n", pw_guid);

    // 3. Get the pre-launched browser
    // When using run-server with ?browser=chromium, the server pre-launches a browser
    let browser_guid = if let Some(pre_browser) = driver.prelaunched_browser() {
        println!("[3] Using pre-launched browser: {}\n", pre_browser);
        pre_browser.to_string()
    } else {
        println!("[3] Launching Chrome (headed)...");
        let guid = driver.launch_browser(false, "chrome").await?;
        println!("    Browser guid: {}\n", guid);
        guid
    };

    // 4. Create a new context and page
    println!("[4] Creating context and page...");
    let context_guid = driver.new_context(&browser_guid).await?;
    let (page_guid, frame_guid) = driver.new_page(&context_guid).await?;
    println!("    Page guid: {}", page_guid);
    println!("    Frame guid: {}\n", frame_guid);

    // 5. Navigate to browserscan
    println!("[5] Navigating to browserscan.net...");
    driver.goto(&frame_guid, "https://www.browserscan.net/bot-detection").await?;
    println!("    Navigation complete!\n");

    // 6. Wait for page to load
    println!("[6] Waiting 10 seconds for page to load...");
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    println!("    Done waiting.\n");

    // 7. Evaluate navigator.webdriver
    println!("[7] Checking navigator.webdriver...");
    let result = driver.evaluate(&frame_guid, "navigator.webdriver").await?;
    println!("    navigator.webdriver = {:?}\n", result);

    // 8. Take a screenshot
    println!("[8] Taking screenshot...");
    let screenshot_b64 = driver.screenshot(&page_guid).await?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(&screenshot_b64)?;
    std::fs::write("test_playwright_result.png", &bytes)?;
    println!("    Screenshot saved: test_playwright_result.png ({} bytes)\n", bytes.len());

    // 9. Close browser (only if we launched it ourselves)
    if driver.prelaunched_browser().is_none() {
        println!("[9] Closing browser...");
        driver.close_browser(&browser_guid).await?;
        println!("    Browser closed.\n");
    } else {
        println!("[9] Skipping browser close (pre-launched).\n");
    }

    println!("=== Test Complete ===");
    Ok(())
}
