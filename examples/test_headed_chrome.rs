use patchright_rs::driver::Driver;
use base64::Engine;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== Patchright HEADED + Chrome Channel Test ===\n");

    // 1. Launch driver
    println!("[1] Launching driver...");
    let mut driver = Driver::launch().await?;
    println!("    Connected!\n");

    // 2. Initialize
    println!("[2] Initializing...");
    let pw_guid = driver.initialize().await?;
    println!("    Playwright: {}", pw_guid);
    println!("    Chromium guid: {:?}", driver.chromium_guid_debug());
    println!();

    // 3. Use the pre-launched browser (headed Chrome via URL params)
    println!("[3] Getting pre-launched browser...");
    let browser_guid = driver.prelaunched_browser()
        .ok_or_else(|| anyhow::anyhow!("No pre-launched browser!"))?
        .to_string();
    println!("    Browser: {}\n", browser_guid);

    // 4. New context + page
    println!("[4] Creating page...");
    let context_guid = driver.new_context(&browser_guid).await?;
    let (page_guid, frame_guid) = driver.new_page(&context_guid).await?;
    println!("    Page: {}, Frame: {}\n", page_guid, frame_guid);

    // 5. Navigate to Browserscan
    println!("[5] Navigating to browserscan.net...");
    driver.goto(&frame_guid, "https://www.browserscan.net/bot-detection").await?;
    println!("    Done!\n");

    // 6. Wait
    println!("[6] Waiting 10s...");
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    // 7. Check webdriver
    println!("[7] Checking navigator.webdriver...");
    let wd = driver.evaluate(&frame_guid, "navigator.webdriver").await?;
    println!("    navigator.webdriver = {:?}\n", wd);

    // 8. Screenshot
    println!("[8] Screenshot...");
    let b64 = driver.screenshot(&page_guid).await?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(&b64)?;
    std::fs::write("bs_headed_chrome.png", &bytes)?;
    println!("    Saved: bs_headed_chrome.png ({} bytes)\n", bytes.len());

    // 9. Done (server manages the pre-launched browser)
    println!("[9] Done!\n");
    println!("=== COMPLETE ===");
    Ok(())
}
