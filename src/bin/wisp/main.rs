mod browser;
mod mcp;
mod scrape;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "wisp",
    version,
    about = "Lightweight undetected browser automation"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Open a URL in headed browser
    Open {
        url: String,
        #[arg(long)]
        headless: bool,
    },
    /// Take a screenshot (default: headless, use --headed to show browser)
    Screenshot {
        url: String,
        #[arg(default_value = "screenshot.png")]
        output: PathBuf,
        #[arg(long)]
        headed: bool,
        #[arg(long, default_value_t = 3000)]
        wait: u64,
    },
    /// Evaluate JavaScript
    Eval {
        expression: String,
        #[arg(long, default_value = "about:blank")]
        url: String,
        #[arg(long)]
        headless: bool,
    },
    /// Dump page text
    Dump {
        url: String,
        #[arg(long)]
        headless: bool,
        #[arg(long, default_value_t = 3000)]
        wait: u64,
    },
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

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("wisp=warn".parse().unwrap()),
        )
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Commands::Open { url, headless } => browser::run_open(url, headless).await,
        Commands::Screenshot {
            url,
            output,
            headed,
            wait,
        } => browser::run_screenshot(url, output, headed, wait).await,
        Commands::Eval {
            expression,
            url,
            headless,
        } => browser::run_eval(expression, url, headless).await,
        Commands::Dump {
            url,
            headless,
            wait,
        } => browser::run_dump(url, headless, wait).await,
        Commands::Scrape {
            url,
            selector,
            format,
        } => scrape::run_scrape(url, selector, format).await,
        Commands::Mcp { cmd } => mcp::run_mcp(cmd).await,
    }
}
