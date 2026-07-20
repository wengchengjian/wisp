use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "patchright",
    version,
    about = "Undetected browser automation CLI - powered by patchright",
    long_about = "A command-line tool for undetected browser automation.\nUses the patchright driver (patched Playwright) for maximum stealth."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install browser binaries (delegates to patchright npm package)
    Install {
        /// Browser to install: chromium, chrome, firefox, webkit
        #[arg(default_value = "chromium")]
        browser: String,
    },

    /// Open a URL in headed browser mode
    Open {
        /// URL to open
        url: String,

        /// Browser channel (chrome, msedge, chromium)
        #[arg(long, default_value = "chrome")]
        channel: String,

        /// Run in headless mode
        #[arg(long)]
        headless: bool,
    },

    /// Take a screenshot of a page
    Screenshot {
        /// URL to screenshot
        url: String,

        /// Output file path (PNG)
        #[arg(default_value = "screenshot.png")]
        output: PathBuf,

        /// Browser channel (chrome, msedge, chromium)
        #[arg(long, default_value = "chrome")]
        channel: String,

        /// Run in headless mode (default: true for screenshot)
        #[arg(long, default_value_t = true)]
        headless: bool,

        /// Wait time in milliseconds before taking screenshot
        #[arg(long, default_value_t = 3000)]
        wait: u64,

        /// Viewport width
        #[arg(long, default_value_t = 1920)]
        width: u32,

        /// Viewport height
        #[arg(long, default_value_t = 1080)]
        height: u32,

        /// Full page screenshot
        #[arg(long)]
        full_page: bool,
    },

    /// Generate a PDF from a page (headless only)
    Pdf {
        /// URL to render as PDF
        url: String,

        /// Output file path (PDF)
        #[arg(default_value = "output.pdf")]
        output: PathBuf,

        /// Browser channel
        #[arg(long, default_value = "chrome")]
        channel: String,

        /// Wait time in milliseconds before generating PDF
        #[arg(long, default_value_t = 3000)]
        wait: u64,
    },

    /// Evaluate JavaScript on a page
    Eval {
        /// JavaScript expression to evaluate
        expression: String,

        /// URL to navigate to first (default: about:blank)
        #[arg(long, default_value = "about:blank")]
        url: String,

        /// Browser channel
        #[arg(long, default_value = "chrome")]
        channel: String,

        /// Run in headless mode
        #[arg(long)]
        headless: bool,
    },

    /// Navigate to a URL and dump page content/text
    Dump {
        /// URL to navigate to
        url: String,

        /// Browser channel
        #[arg(long, default_value = "chrome")]
        channel: String,

        /// Run in headless mode
        #[arg(long)]
        headless: bool,

        /// Wait time in milliseconds
        #[arg(long, default_value_t = 3000)]
        wait: u64,

        /// Output only text content (strip HTML)
        #[arg(long)]
        text: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing (only show warnings/errors by default)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("patchright_rs=warn".parse().unwrap()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Install { browser } => cmd_install(&browser).await,
        Commands::Open { url, channel, headless } => cmd_open(&url, &channel, headless).await,
        Commands::Screenshot { url, output, channel, headless, wait, width, height, full_page } => {
            cmd_screenshot(&url, &output, &channel, headless, wait, width, height, full_page).await
        }
        Commands::Pdf { url, output, channel, wait } => {
            cmd_pdf(&url, &output, &channel, wait).await
        }
        Commands::Eval { expression, url, channel, headless } => {
            cmd_eval(&expression, &url, &channel, headless).await
        }
        Commands::Dump { url, channel, headless, wait, text } => {
            cmd_dump(&url, &channel, headless, wait, text).await
        }
    }
}

/// Install browsers via the patchright npm package
async fn cmd_install(browser: &str) -> anyhow::Result<()> {
    println!("Installing {browser}...");

    let status = std::process::Command::new("npx")
        .arg("patchright")
        .arg("install")
        .arg(browser)
        .status()?;

    if status.success() {
        println!("✓ {browser} installed successfully");
    } else {
        anyhow::bail!("Installation failed with exit code: {:?}", status.code());
    }
    Ok(())
}

/// Open a URL in a headed browser window
async fn cmd_open(url: &str, channel: &str, headless: bool) -> anyhow::Result<()> {
    println!("Opening {url} (channel={channel}, headless={headless})...");

    let mut driver = launch_driver(headless, channel).await?;
    let (_page_guid, frame_guid) = create_page(&mut driver).await?;

    driver.goto(&frame_guid, url).await?;
    println!("✓ Page loaded: {url}");
    println!("  Browser is open. Press Ctrl+C to close.");

    // Keep the browser open until Ctrl+C
    tokio::signal::ctrl_c().await?;
    println!("\nClosing...");

    Ok(())
}

/// Take a screenshot
async fn cmd_screenshot(
    url: &str,
    output: &PathBuf,
    channel: &str,
    headless: bool,
    wait: u64,
    width: u32,
    height: u32,
    full_page: bool,
) -> anyhow::Result<()> {
    println!("Screenshot: {url} -> {}", output.display());

    let mut driver = launch_driver(headless, channel).await?;
    let (page_guid, frame_guid) = create_page(&mut driver).await?;

    // Set viewport size
    driver.conn.send_command(&page_guid, "setViewportSize", serde_json::json!({
        "viewportSize": { "width": width, "height": height }
    })).await?;

    driver.goto(&frame_guid, url).await?;

    if wait > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
    }

    // Take screenshot
    let result = driver.conn.send_command(&page_guid, "screenshot", serde_json::json!({
        "type": "png",
        "fullPage": full_page,
        "timeout": 30000
    })).await?;

    let binary = result.get("binary")
        .and_then(|b| b.as_str())
        .ok_or_else(|| anyhow::anyhow!("No screenshot data in response"))?;

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(binary)?;
    std::fs::write(output, &bytes)?;

    println!("✓ Screenshot saved: {} ({} bytes)", output.display(), bytes.len());
    Ok(())
}

/// Generate PDF
async fn cmd_pdf(url: &str, output: &PathBuf, channel: &str, wait: u64) -> anyhow::Result<()> {
    println!("PDF: {url} -> {}", output.display());

    // PDF requires headless mode
    let mut driver = launch_driver(true, channel).await?;
    let (page_guid, frame_guid) = create_page(&mut driver).await?;

    driver.goto(&frame_guid, url).await?;

    if wait > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
    }

    // Generate PDF
    let result = driver.conn.send_command(&page_guid, "pdf", serde_json::json!({
        "timeout": 30000
    })).await?;

    let binary = result.get("pdf")
        .and_then(|b| b.as_str())
        .ok_or_else(|| anyhow::anyhow!("No PDF data in response"))?;

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(binary)?;
    std::fs::write(output, &bytes)?;

    println!("✓ PDF saved: {} ({} bytes)", output.display(), bytes.len());
    Ok(())
}

/// Evaluate JavaScript
async fn cmd_eval(expression: &str, url: &str, channel: &str, headless: bool) -> anyhow::Result<()> {
    let mut driver = launch_driver(headless, channel).await?;
    let (_page_guid, frame_guid) = create_page(&mut driver).await?;

    if url != "about:blank" {
        driver.goto(&frame_guid, url).await?;
    }

    let result = driver.evaluate(&frame_guid, expression).await?;

    // Extract the value from the Playwright response
    let value = result.get("value")
        .map(|v| {
            // Playwright serializes values in a specific format
            if let Some(obj) = v.as_object() {
                if obj.contains_key("v") {
                    return obj["v"].clone();
                }
                if obj.contains_key("n") {
                    return obj["n"].clone();
                }
                if obj.contains_key("s") {
                    return obj["s"].clone();
                }
                if obj.contains_key("b") {
                    return obj["b"].clone();
                }
            }
            v.clone()
        })
        .unwrap_or(serde_json::Value::Null);

    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// Dump page content
async fn cmd_dump(url: &str, channel: &str, headless: bool, wait: u64, text: bool) -> anyhow::Result<()> {
    let mut driver = launch_driver(headless, channel).await?;
    let (_page_guid, frame_guid) = create_page(&mut driver).await?;

    driver.goto(&frame_guid, url).await?;

    if wait > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
    }

    let expression = if text {
        "document.body.innerText"
    } else {
        "document.documentElement.outerHTML"
    };

    let result = driver.evaluate(&frame_guid, expression).await?;

    let content = result.get("value")
        .and_then(|v| v.get("s").or_else(|| v.get("v")))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    println!("{content}");
    Ok(())
}

// --- Helper functions ---

/// Launch the patchright driver with specified settings
async fn launch_driver(headless: bool, channel: &str) -> anyhow::Result<patchright_rs::driver::Driver> {
    let mut driver = patchright_rs::driver::Driver::launch_with_options(headless, channel).await?;
    driver.initialize().await?;
    Ok(driver)
}

/// Create a new page using the pre-launched browser
async fn create_page(driver: &mut patchright_rs::driver::Driver) -> anyhow::Result<(String, String)> {
    let browser_guid = driver.prelaunched_browser()
        .ok_or_else(|| anyhow::anyhow!("No pre-launched browser available"))?
        .to_string();

    let context_guid = driver.new_context(&browser_guid).await?;
    let (page_guid, frame_guid) = driver.new_page(&context_guid).await?;
    Ok((page_guid, frame_guid))
}
