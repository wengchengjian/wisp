use std::time::{Duration, Instant};
use wisp::{Browser, LaunchOptions};
use wisp::challenge::ChallengeSolver;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== CF Turnstile Solve Test ===");
    let start = Instant::now();

    let browser = Browser::launch(LaunchOptions {
        headless: false,
        ..Default::default()
    }).await?;
    println!("[{:.1}s] Browser launched", start.elapsed().as_secs_f64());

    let page = browser.new_page().await?;
    page.goto("https://www.bz555555555.com").await?;
    println!("[{:.1}s] Navigated", start.elapsed().as_secs_f64());

    // Wait for JS challenge to transition to Turnstile
    tokio::time::sleep(Duration::from_secs(6)).await;

    // Check for Turnstile iframe
    let iframe_info = page.evaluate(r#"(() => {
        const iframe = document.querySelector('iframe[src*="challenges.cloudflare.com"]');
        if (!iframe) return JSON.stringify({found: false});
        const rect = iframe.getBoundingClientRect();
        return JSON.stringify({found: true, x: rect.x, y: rect.y, w: rect.width, h: rect.height, src: iframe.src.substring(0, 100)});
    })()"#).await?;
    println!("[{:.1}s] Turnstile iframe: {}", start.elapsed().as_secs_f64(), iframe_info.as_str().unwrap_or("null"));

    // Try to solve using ChallengeSolver
    println!("[{:.1}s] Attempting solve (60s timeout)...", start.elapsed().as_secs_f64());
    let solver = ChallengeSolver::new(&page);
    match solver.solve(Duration::from_secs(60)).await {
        Ok(()) => println!("[{:.1}s] SOLVE OK!", start.elapsed().as_secs_f64()),
        Err(e) => println!("[{:.1}s] SOLVE FAILED: {}", start.elapsed().as_secs_f64(), e),
    }

    // Check final state
    let title = page.evaluate_as_string("document.title").await?;
    let html = page.evaluate_as_string("document.body.innerText").await.unwrap_or_default();
    println!();
    println!("=== FINAL ===");
    println!("Time: {:.1}s", start.elapsed().as_secs_f64());
    println!("Title: {}", title);
    println!("Body text (first 200): {}", &html[..html.len().min(200)]);

    browser.close().await?;
    Ok(())
}
