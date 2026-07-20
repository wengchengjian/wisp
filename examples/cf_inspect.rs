use std::time::Duration;
use wisp::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let browser = Browser::launch(LaunchOptions { headless: false, ..Default::default() }).await?;
    let page = browser.new_page().await?;
    page.goto("https://www.bz555555555.com").await?;
    tokio::time::sleep(Duration::from_secs(8)).await;

    // Dump page structure for analysis
    let info = page.evaluate(r#"(() => {
        const iframes = Array.from(document.querySelectorAll('iframe')).map(f => ({
            src: (f.src || '').substring(0, 150),
            id: f.id,
            class: f.className,
            w: f.getBoundingClientRect().width,
            h: f.getBoundingClientRect().height
        }));
        const cfElements = Array.from(document.querySelectorAll('[class*="cf-"], [id*="cf-"], [id*="challenge"], [class*="challenge"]')).map(e => ({
            tag: e.tagName,
            id: e.id,
            class: e.className.substring(0, 100)
        }));
        return JSON.stringify({
            title: document.title,
            iframes: iframes,
            cfElements: cfElements,
            bodyClasses: document.body.className,
            scripts: Array.from(document.querySelectorAll('script[src]')).map(s => s.src.substring(0, 100)).slice(0, 5)
        }, null, 2);
    })()"#).await?;
    println!("{}", info.as_str().unwrap_or("null"));

    browser.close().await?;
    Ok(())
}
