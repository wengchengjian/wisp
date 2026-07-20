use patchright_rs::{Browser, LaunchOptions};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-tostring-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions { headless: true, user_data_dir: Some(user_data.clone()), ..Default::default() }).await?;
    let page = browser.new_page().await?;
    let html = r#"data:text/html,<script>
        const desc = Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver');
        document.title = JSON.stringify({
            get_tostring: desc.get.toString(),
            typeof_wd: typeof navigator.webdriver,
            configurable: desc.configurable,
            enumerable: desc.enumerable,
            permissions_tostring: navigator.permissions.query.toString().substring(0, 40),
        });
    </script>"#;
    page.goto(html).await?;
    println!("{}", page.evaluate_as_string("document.title").await?);
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
