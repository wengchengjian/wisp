use patchright_rs::driver::Driver;
use base64::Engine;

/// Basic example: launch browser, navigate, evaluate JS, take screenshot.
/// Uses the patchright driver (Playwright protocol) for maximum stealth.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Launch the patchright driver (headed Chrome for maximum stealth)
    println!("Launching browser...");
    let mut driver = Driver::launch().await?;
    driver.initialize().await?;

    // Get the pre-launched browser
    let browser_guid = driver.prelaunched_browser()
        .ok_or("No pre-launched browser")?
        .to_string();

    // Create a new page
    let context_guid = driver.new_context(&browser_guid).await?;
    let (page_guid, frame_guid) = driver.new_page(&context_guid).await?;

    // Navigate
    println!("Navigating to example.com...");
    driver.goto(&frame_guid, "https://example.com").await?;

    // Evaluate JavaScript
    let result = driver.evaluate(&frame_guid, "navigator.webdriver").await?;
    println!("navigator.webdriver = {:?}", result.get("value"));

    let title = driver.evaluate(&frame_guid, "document.title").await?;
    println!("Page title: {:?}", title.get("value"));

    // Screenshot
    let screenshot_b64 = driver.screenshot(&page_guid).await?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(&screenshot_b64)?;
    std::fs::write("example.png", &bytes)?;
    println!("Screenshot saved to example.png ({} bytes)", bytes.len());

    println!("Done!");
    Ok(())
}
