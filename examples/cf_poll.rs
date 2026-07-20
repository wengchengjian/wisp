use std::time::Duration;
use wisp::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let browser = Browser::launch(LaunchOptions { headless: false, ..Default::default() }).await?;
    let page = browser.new_page().await?;
    page.goto("https://www.bz555555555.com").await?;

    for i in 0..10u64 {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let state = page.evaluate(r#"(() => {
            const iframes = document.querySelectorAll('iframe');
            const cfIframes = Array.from(iframes).filter(f => f.src && f.src.includes('challenges'));
            const widget = document.querySelector('[id*="cf-chl-widget"]');
            const responseInput = document.querySelector('[name="cf-turnstile-response"], [id*="_response"]');
            return JSON.stringify({
                title: document.title,
                totalIframes: iframes.length,
                cfIframes: cfIframes.map(f => ({src: f.src.substring(0,80), vis: f.offsetHeight > 0})),
                widgetId: widget ? widget.id : null,
                responseValue: responseInput ? (responseInput.value || '').substring(0, 30) : null,
            });
        })()"#).await?;
        println!("[{}s] {}", (i+1)*3, state.as_str().unwrap_or(""));

        let title = page.evaluate_as_string("document.title").await.unwrap_or_default();
        if !title.contains("Just a moment") && !title.is_empty() && state.as_str().map_or(true, |s| !s.contains("cf-chl-widget")) {
            println!("PASSED! Title: {}", title);
            let body = page.evaluate_as_string("document.body.innerText").await.unwrap_or_default();
            println!("Body: {}", &body[..body.len().min(300)]);
            break;
        }
    }

    browser.close().await?;
    Ok(())
}
