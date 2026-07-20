use patchright_rs::{Browser, LaunchOptions};

#[tokio::main]
async fn main() {
    let user_data = std::env::temp_dir().join(format!("patchright-test-{}", std::process::id()));
    
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        user_data_dir: Some(user_data.clone()),
        ..Default::default()
    }).await.unwrap();

    let page = browser.new_page().await.unwrap();
    // Page script writes detection results to DOM (shared between worlds)
    page.goto("data:text/html,<script>document.title = JSON.stringify({webdriver: navigator.webdriver, typeof_wd: typeof navigator.webdriver, plugins: navigator.plugins.length, chrome: typeof window.chrome !== 'undefined', languages: navigator.languages.length});</script>").await.unwrap();

    // Read document.title from isolated world (DOM is shared)
    let title = page.evaluate_as_string("document.title").await.unwrap();
    println!("Main world detection results: {title}");
    
    browser.close().await.unwrap();
    let _ = std::fs::remove_dir_all(&user_data);
}
