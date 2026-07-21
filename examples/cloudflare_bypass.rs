//! Example: Bypass Cloudflare protection using Fetcher stealth mode.

use std::time::Duration;
use wisp::Fetcher;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Scraping https://nowsecure.nl ...");

    let resp = Fetcher::stealth()
        .headless(true)
        .human_mode(true)
        .challenge_timeout(Duration::from_secs(30))
        // Optional: add proxy
        // .proxy("http://user:pass@proxy:8080")
        .get("https://nowsecure.nl")
        .await?;

    println!("Status: {}", resp.status);
    println!("URL: {}", resp.url);
    println!("Title: {:?}", resp.title);
    println!("Body length: {} bytes", resp.body.len());

    let html = resp.text()?;
    if html.contains("nowsecure") && !html.contains("Just a moment") {
        println!("\n[OK] Cloudflare bypass successful!");
    } else {
        println!("\n[WARN] May not have passed Cloudflare challenge");
    }

    Ok(())
}
