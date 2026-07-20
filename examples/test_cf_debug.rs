use std::time::{Duration, Instant};
use wisp::{Browser, LaunchOptions};
use wisp::challenge::{ChallengeSolver, ChallengeType};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== CF Test: headed mode, 60s timeout ===");
    let start = Instant::now();

    let browser = Browser::launch(LaunchOptions {
        headless: false,  // HEADED for maximum stealth
        ..Default::default()
    }).await?;
    println!("[{:.1}s] Browser launched", start.elapsed().as_secs_f64());

    let page = browser.new_page().await?;
    page.goto("https://www.bz555555555.com").await?;
    println!("[{:.1}s] Navigated", start.elapsed().as_secs_f64());

    // Check what challenge we see
    let solver = ChallengeSolver::new(&page);
    let challenge = solver.detect().await?;
    println!("[{:.1}s] Challenge detected: {:?}", start.elapsed().as_secs_f64(), challenge);

    // Get page title during challenge
    let title = page.evaluate_as_string("document.title").await?;
    println!("[{:.1}s] Title: {}", start.elapsed().as_secs_f64(), title);

    // Wait and poll
    for i in 0..12 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let title = page.evaluate_as_string("document.title").await.unwrap_or_default();
        let url = page.evaluate_as_string("window.location.href").await.unwrap_or_default();
        let challenge = solver.detect().await.unwrap_or(ChallengeType::None);
        println!("[{:.1}s] Poll {}: title='{}' challenge={:?}", start.elapsed().as_secs_f64(), i+1, title, challenge);

        if challenge == ChallengeType::None && !title.contains("Just a moment") {
            println!("[{:.1}s] Challenge PASSED!", start.elapsed().as_secs_f64());
            break;
        }
    }

    // Final state
    let html = page.evaluate_as_string("document.documentElement.outerHTML").await?;
    let title = page.evaluate_as_string("document.title").await?;
    println!();
    println!("=== RESULT ===");
    println!("Total time: {:.2}s", start.elapsed().as_secs_f64());
    println!("Title: {}", title);
    println!("HTML length: {} bytes", html.len());

    let blocked = html.contains("Just a moment") || html.contains("challenge-platform");
    if blocked {
        println!("[FAIL] Still on challenge page");
    } else {
        println!("[OK] Got real content!");
        let preview: String = html.chars().take(200).collect();
        println!("Preview: {}", preview);
    }

    browser.close().await?;
    Ok(())
}
