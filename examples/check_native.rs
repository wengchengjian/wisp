use patchright_rs::{Browser, LaunchOptions};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-native-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions { headless: true, user_data_dir: Some(user_data.clone()), ..Default::default() }).await?;
    let page = browser.new_page().await?;
    // Check if the NATIVE webdriver (before our JS patch) is already undefined
    // by checking the property descriptor
    let html = r#"data:text/html,<script>
        const desc = Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver');
        document.title = JSON.stringify({
            has_descriptor: !!desc,
            configurable: desc ? desc.configurable : null,
            has_get: desc ? !!desc.get : null,
            get_tostring: desc && desc.get ? desc.get.toString().substring(0, 50) : null,
            value: navigator.webdriver,
            typeof_value: typeof navigator.webdriver,
        });
    </script>"#;
    page.goto(html).await?;
    println!("{}", page.evaluate_as_string("document.title").await?);
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
