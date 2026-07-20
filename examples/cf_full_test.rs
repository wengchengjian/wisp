use std::time::{Duration, Instant};
use wisp::{Browser, LaunchOptions};
use wisp::challenge::ChallengeSolver;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== CF Full Solve + Wait Test ===");
    let start = Instant::now();

    let browser = Browser::launch(LaunchOptions { headless: false, ..Default::default() }).await?;
    let page = browser.new_page().await?;
    page.goto("https://www.bz555555555.com").await?;
    println!("[{:.1}s] Navigated", start.elapsed().as_secs_f64());

    // Wait for JS challenge to become Turnstile
    tokio::time::sleep(Duration::from_secs(6)).await;
    println!("[{:.1}s] Waiting for Turnstile...", start.elapsed().as_secs_f64());

    // Solve
    let solver = ChallengeSolver::new(&page);
    match solver.solve(Duration::from_secs(30)).await {
        Ok(()) => println!("[{:.1}s] Solver returned OK", start.elapsed().as_secs_f64()),
        Err(e) => println!("[{:.1}s] Solver error: {}", start.elapsed().as_secs_f64(), e),
    }

    // After solve, wait for page to actually navigate/load real content
    println!("[{:.1}s] Waiting for real content...", start.elapsed().as_secs_f64());
    for i in 0..20 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let title = page.evaluate_as_string("document.title").await.unwrap_or_default();
        let url = page.evaluate_as_string("window.location.href").await.unwrap_or_default();
        let has_cf = page.evaluate(r#"(() => {
            return document.body.innerHTML.includes('cf-chl-widget') ||
                   document.body.innerHTML.includes('challenge-platform') ||
                   document.title.includes('Just a moment');
        })()"#).await.map(|v| v.as_bool().unwrap_or(false)).unwrap_or(false);

        println!("[{:.1}s] #{} title='{}' cf={} url={}", start.elapsed().as_secs_f64(), i+1, title, has_cf, &url[..url.len().min(50)]);

        if !has_cf && !title.is_empty() {
            println!("\n=== PASSED ===");
            println!("[{:.1}s] Total time", start.elapsed().as_secs_f64());
            println!("Title: {}", title);
            let body = page.evaluate_as_string("document.body.innerText").await.unwrap_or_default();
            println!("Body (300 chars): {}", &body[..body.len().min(300)]);
            break;
        }
    }

    browser.close().await?;
    Ok(())
}
