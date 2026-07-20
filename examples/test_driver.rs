use patchright_rs::driver::Driver;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Launching patchright driver...");
    let mut driver = Driver::launch().await?;
    println!("Driver connected!");

    // Initialize Playwright
    let result = driver.initialize().await?;
    println!("Initialized: {:?}", result);

    Ok(())
}
