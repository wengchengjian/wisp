# wisp

Lightweight undetected browser automation for Rust — built for scraping.

Pure Rust CDP (Chrome DevTools Protocol) over WebSocket with anti-detection patches. Passes Browserscan 4/4 in both headed and headless modes.

## Features

- **Unified Fetcher API** — One interface, three modes: `Http` (TLS fingerprint), `Dynamic` (JS rendering), `Stealth` (Cloudflare bypass)
- **Anti-Detection** — Patches navigator.webdriver, CDP leaks, automation flags; passes Browserscan 4/4
- **Cloudflare Bypass** — Auto-detects and solves JS Challenge, Turnstile, and Managed Challenge
- **Adaptive Parsing** — CSS/XPath selectors + element relocation when site structure changes
- **Spider Engine** — Concurrent crawling with priority scheduling, robots.txt, checkpoint/resume, streaming
- **Human Simulation** — Bezier mouse movements, random scrolling, human-like typing
- **Proxy Rotation** — Built-in pool with Sequential/Random/Sticky strategies
- **MCP Server** — AI-assisted scraping via Model Context Protocol (stdio JSON-RPC)

## Quick Start

```rust
use wisp::Fetcher;

#[tokio::main]
async fn main() -> wisp::Result<()> {
    // Fast HTTP with TLS fingerprint (milliseconds, no browser)
    let page = Fetcher::http().get("https://quotes.toscrape.com/").await?;
    let quotes = page.css(".quote .text");
    println!("Found {} quotes", quotes.len());

    // JS rendering (seconds, Chromium)
    let page = Fetcher::dynamic()
        .headless(true)
        .wait_for(".content")
        .get("https://spa-example.com/")
        .await?;

    // Stealth mode — bypass Cloudflare (seconds, anti-detection browser)
    let page = Fetcher::stealth()
        .proxy("http://127.0.0.1:7897")
        .challenge_timeout(std::time::Duration::from_secs(60))
        .get("https://cf-protected-site.com/")
        .await?;

    // Unified parsing API — works identically across all modes
    let titles = page.css("h1");
    let items = page.xpath("//div[@class='item']");
    let found = page.find_by_text("Hello", Some("p"), true);
    let next_req = page.follow("/page/2/");

    Ok(())
}
```

## Three Fetcher Modes

| Mode | Use Case | Engine | Cost |
|------|----------|--------|------|
| `Fetcher::http()` | Static HTML, APIs, no anti-bot | wreq + TLS fingerprint (JA3/JA4) | Lowest (ms) |
| `Fetcher::dynamic()` | JavaScript-rendered pages (SPA) | CDP + Chromium | Medium (s) |
| `Fetcher::stealth()` | Cloudflare / strong anti-bot | CDP + anti-detection patches + CF solver | Highest (s) |

All three modes return the same `Response` type with identical parsing APIs.

## Session (Persistent Cookies)

```rust
use wisp::Session;

let session = Session::stealth()
    .proxy("http://127.0.0.1:7897")
    .build()?;

let _ = session.get("https://site.com/login").await?;
let dashboard = session.get("https://site.com/dashboard").await?; // cookies preserved
session.close().await;
```

## Spider Crawling

```rust
use wisp::{SpiderBuilder, Engine, FetchMode};
use serde_json::json;

let spider = SpiderBuilder::new("quotes")
    .start_urls(vec!["https://quotes.toscrape.com/"])
    .mode(FetchMode::Http)
    .concurrent(8)
    .delay_ms(200)
    .obey_robots(false)
    .parse(|resp| {
        let items = resp.css(".quote").iter().map(|q| {
            json!({
                "text": q.select_one(".text").map(|n| n.text()).unwrap_or_default(),
                "author": q.select_one(".author").map(|n| n.text()).unwrap_or_default(),
            })
        }).collect();

        let follows = resp.select_one(".next a")
            .and_then(|a| a.attr("href"))
            .and_then(|href| resp.follow(&href))
            .map(|r| vec![r])
            .unwrap_or_default();

        (items, follows)
    })
    .build();

let stats = Engine::builder(spider)
    .max_pages(10)
    .proxy_pool(vec!["http://127.0.0.1:7897".into()], wisp::RotationStrategy::Sequential)
    .run()
    .await?;

println!("{}", stats.summary());
```

## Architecture

```
src/
  fetcher/          Unified entry point (Fetcher + Response + Session)
    mod.rs          Fetcher struct, FetchMode enum, FetcherBuilder
    response.rs     Unified Response/Request types
    session.rs      Session with cookie persistence
  parser/           HTML parsing (CSS/XPath/adaptive/text search)
  crawl/            Spider engine (scheduler, robots, checkpoint, streaming)
  browser/          CDP browser automation (internal)
  challenge/        Cloudflare challenge solver (internal)
  human/            Human behavior simulation (internal)
  fetch/            HTTP client with TLS emulation (internal)
  proxy/            Proxy pool with rotation strategies
  storage/          SQLite storage (adaptive snapshots, checkpoints, cache)
  mcp/              MCP server for AI-assisted scraping
```

## Builder Options

```rust
Fetcher::stealth()
    .proxy("http://host:port")          // Proxy
    .timeout(Duration::from_secs(60))   // Request timeout
    .headless(true)                     // Headless browser
    .emulation(Profile::Chrome136)      // TLS fingerprint (Http mode)
    .human_mode(true)                   // Human behavior simulation
    .challenge_timeout(Duration::from_secs(30))  // CF solve timeout
    .wait_for(".content")               // Wait for selector
    .extra_wait_ms(1000)                // Extra wait after load
    .block_ads()                        // Block ~200 ad domains
    .block_domains(&["analytics.com"])  // Block specific domains
    .dns_over_https("https://1.1.1.1/dns-query")  // DoH (anti DNS leak)
    .build();
```

## Running Tests

```bash
# Unit tests (no network required)
cargo test --lib

# Builder & unified API tests
cargo test --test builder_api_test --test unified_fetcher_test

# Real network tests (requires internet, uses proxy 127.0.0.1:7897)
cargo test --test cf_bypass_real_test -- --ignored
cargo test --test real_scrape_test -- --ignored
cargo test --test session_test -- --ignored
```

## Requirements

- Rust 1.80+ (edition 2021)
- Chrome/Chromium (for Dynamic/Stealth modes)
- Network access (for real tests; proxy `127.0.0.1:7897` supported)

## License

Apache-2.0
