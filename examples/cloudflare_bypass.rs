//! Example: Bypass Cloudflare protection and scrape a protected page.

use std::time::Duration;
use wisp::scraper::Scraper;
use wisp::proxy::RotationStrategy;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Build a scraper with anti-Cloudflare capabilities
    let scraper = Scraper::builder()
        .headless(true)
        .human_mode(true)
        .challenge_timeout(Duration::from_secs(30))
        .max_retries(2)
        // Optional: add proxies
        // .proxies(vec!["http://user:pass@proxy:8080".into()])
        // .proxy_strategy(RotationStrategy::Random)
        .build()?;

    // Scrape a Cloudflare-protected page
    println!("Scraping https://nowsecure.nl ...");
    let resp = scraper.get("https://nowsecure.nl").await?;

    println!("Status: {}", resp.status);
    println!("URL: {}", resp.url);
    println!("Title: {}", resp.title);
    println!("HTML length: {} bytes", resp.html.len());

    // Check if we passed Cloudflare
    if resp.html.contains("nowsecure") && !resp.html.contains("Just a moment") {
        println!("\n[OK] Cloudflare bypass successful!");
    } else {
        println!("\n[WARN] May not have passed Cloudflare challenge");
    }

    Ok(())
}
