use patchright_rs::{Browser, LaunchOptions};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-adv-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions { headless: false, user_data_dir: Some(user_data.clone()), ..Default::default() }).await?;
    let page = browser.new_page().await?;
    // Advanced detection checks that Browserscan might use
    let html = r#"data:text/html,<script>
        const wd_desc = Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver');
        const plugins_desc = Object.getOwnPropertyDescriptor(Navigator.prototype, 'plugins');
        document.title = JSON.stringify({
            // Check getter name (native is 'get webdriver')
            wd_getter_name: wd_desc.get.name,
            // Check if getter constructor matches native
            wd_getter_constructor: wd_desc.get.constructor.name,
            plugins_getter_constructor: plugins_desc.get.constructor.name,
            // Check if both getters have same prototype
            same_proto: Object.getPrototypeOf(wd_desc.get) === Object.getPrototypeOf(plugins_desc.get),
            // Check toString
            wd_tostring: Function.prototype.toString.call(wd_desc.get),
            plugins_tostring: Function.prototype.toString.call(plugins_desc.get).substring(0, 50),
            // Check length property
            wd_length: wd_desc.get.length,
            plugins_length: plugins_desc.get.length,
        });
    </script>"#;
    page.goto(html).await?;
    println!("{}", page.evaluate_as_string("document.title").await?);
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
