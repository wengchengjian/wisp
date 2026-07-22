use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "wisp", version, about = "Lightweight undetected browser automation")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Open a URL in headed browser
    Open { url: String, #[arg(long)] headless: bool },
    /// Take a screenshot (default: headless, use --headed to show browser)
    Screenshot { url: String, #[arg(default_value = "screenshot.png")] output: PathBuf, #[arg(long)] headed: bool, #[arg(long, default_value_t = 3000)] wait: u64 },
    /// Evaluate JavaScript
    Eval { expression: String, #[arg(long, default_value = "about:blank")] url: String, #[arg(long)] headless: bool },
    /// Dump page text
    Dump { url: String, #[arg(long)] headless: bool, #[arg(long, default_value_t = 3000)] wait: u64 },
    /// Scrape a URL with CSS selector (HTTP fetch, no browser)
    Scrape {
        url: String,
        #[arg(long)]
        selector: Option<String>,
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// MCP server commands
    Mcp {
        #[command(subcommand)]
        cmd: McpCmd,
    },
}

#[derive(Subcommand)]
enum McpCmd {
    /// 启动 stdio MCP server
    Serve {
        #[arg(long, default_value = "./wisp.db")]
        db: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("wisp=warn".parse().unwrap()))
        .with_target(false)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Open { url, headless } => {
            use wisp::{Browser, LaunchOptions};
            println!("Opening {url}...");
            let browser = Browser::launch(LaunchOptions { headless, ..Default::default() }).await?;
            let mut page = browser.new_page().await?;
            page.goto(&url).await?;
            println!("✓ Page loaded. Press Ctrl+C to close.");
            tokio::signal::ctrl_c().await?;
            browser.close().await?;
        }
        Commands::Screenshot { url, output, headed, wait } => {
            use wisp::{Browser, LaunchOptions};
            println!("Screenshot: {url}");
            let browser = Browser::launch(LaunchOptions { headless: !headed, ..Default::default() }).await?;
            let mut page = browser.new_page().await?;
            page.goto(&url).await?;
            if wait > 0 { tokio::time::sleep(std::time::Duration::from_millis(wait)).await; }
            page.screenshot(output.to_str().unwrap_or("screenshot.png")).await?;
            println!("✓ Saved: {}", output.display());
            browser.close().await?;
        }
        Commands::Eval { expression, url, headless } => {
            use wisp::{Browser, LaunchOptions};
            let browser = Browser::launch(LaunchOptions { headless, ..Default::default() }).await?;
            let mut page = browser.new_page().await?;
            if url != "about:blank" { page.goto(&url).await?; }
            let result = page.evaluate(&expression).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            browser.close().await?;
        }
        Commands::Dump { url, headless, wait } => {
            use wisp::{Browser, LaunchOptions};
            let browser = Browser::launch(LaunchOptions { headless, ..Default::default() }).await?;
            let mut page = browser.new_page().await?;
            page.goto(&url).await?;
            if wait > 0 { tokio::time::sleep(std::time::Duration::from_millis(wait)).await; }
            let text = page.evaluate_as_string("document.body.innerText").await?;
            println!("{text}");
            browser.close().await?;
        }
        Commands::Scrape { url, selector, format } => {
            use wisp::http::Client;
            let client = Client::builder().build()?;
            let resp = client.get(&url).await?;
            let html = resp.text()?;
            if let Some(sel) = selector {
                use wisp::parser::Node;
                let doc = Node::from_html(&html);
                let nodes = doc.select(&sel);
                let items: Vec<serde_json::Value> = nodes.iter()
                    .map(|n| serde_json::json!({"text": n.text()}))
                    .collect();
                match format.as_str() {
                    "jsonl" => {
                        for item in &items {
                            println!("{}", serde_json::to_string(item)?);
                        }
                    }
                    _ => println!("{}", serde_json::to_string_pretty(&items)?),
                }
            } else {
                println!("{html}");
            }
        }
        Commands::Mcp { cmd } => match cmd {
            McpCmd::Serve { db } => {
                let store = if db == ":memory:" {
                    Arc::new(wisp::Store::open_in_memory()?)
                } else {
                    Arc::new(wisp::Store::open(std::path::Path::new(&db))?)
                };
                wisp::mcp::serve(store).await?;
            }
        },
    }
    Ok(())
}
