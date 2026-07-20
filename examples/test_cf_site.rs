use std::time::{Duration, Instant};
use wisp::scraper::Scraper;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let scraper = Scraper::builder()
        .headless(true)
        .human_mode(false)  // disable for speed test
        .challenge_timeout(Duration::from_secs(30))
        .max_retries(1)
        .build()?;

    println!("=== Testing https://www.bz555555555.com ===");
    println!("Mode: headless, no human simulation");
    println!();

    let start = Instant::now();
    let resp = scraper.get("https://www.bz555555555.com").await?;
    let elapsed = start.elapsed();

    println!("Total time: {:.2}s", elapsed.as_secs_f64());
    println!("Status: {}", resp.status);
    println!("Final URL: {}", resp.url);
    println!("Title: {}", resp.title);
    println!("HTML length: {} bytes", resp.html.len());
    println!("Cookies: {:?}", resp.cookies);
    println!();

    // Check if CF blocked us
    let blocked = resp.html.contains("Just a moment") ||
                  resp.html.contains("challenge-platform") ||
                  resp.html.contains("cf-browser-verification") ||
                  resp.status == 403;

    if blocked {
        println!("[FAIL] Cloudflare challenge NOT passed");
        // Print first 500 chars for debugging
        println!("HTML preview: {}", &resp.html[..resp.html.len().min(500)]);
    } else {
        println!("[OK] Page loaded successfully!");
        // Print title and first 200 chars of body text
        let body_preview: String = resp.html.chars().take(300).collect();
        println!("Preview: {}...", body_preview);
    }

    Ok(())
}
