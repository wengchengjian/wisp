use patchright_rs::{Browser, LaunchOptions};
use tokio::io::AsyncBufReadExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-dbg7-{}", std::process::id()));
    println!("Launching...");
    let browser = Browser::launch(LaunchOptions {
        headless: true,
        user_data_dir: Some(user_data.clone()),
        ..Default::default()
    }).await?;
    println!("Launched! Testing CDP...");

    // Try a simple CDP command with timeout
    let session = &browser.session;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        session.execute("Browser.getVersion", serde_json::json!({}))
    ).await;

    match result {
        Ok(Ok(v)) => println!("CDP RESPONSE: {v}"),
        Ok(Err(e)) => println!("CDP ERROR: {e}"),
        Err(_) => println!("CDP TIMEOUT - no response from Chrome"),
    }

    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
