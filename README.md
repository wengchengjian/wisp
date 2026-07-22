# wisp

Lightweight undetected browser automation for Rust — built for scraping.

Pure Rust CDP (Chrome DevTools Protocol) over WebSocket with anti-detection patches. Passes Browserscan 4/4 in both headed and headless modes.

## Features

- **Unified Fetcher API** — One interface, three modes: `Http` (TLS fingerprint), `Dynamic` (JS rendering), `Stealth` (Cloudflare bypass)
- **Anti-Detection** — Patches navigator.webdriver, CDP leaks, automation flags; passes Browserscan 4/4
- **Cloudflare Bypass** — Auto-detects and solves JS Challenge, Turnstile, and Managed Challenge
- **Adaptive Parsing** — CSS/XPath selectors + element relocation when site structure changes
- **Spider Engine** — Concurrent crawling with priority scheduling, callback label routing, per-spider stop conditions, checkpoint/resume, streaming
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
use wisp::crawl::SpiderRequest;
use serde_json::json;

let spider = SpiderBuilder::new("quotes")
    .start_urls(vec!["https://quotes.toscrape.com/"])
    .mode(FetchMode::Http)
    .delay_ms(200)
    .obey_robots(false)
    .on("default", |resp| async move {
        let items = resp.css(".quote").iter().map(|q| {
            json!({
                "text": q.select_one(".text").map(|n| n.text()).unwrap_or_default(),
                "author": q.select_one(".author").map(|n| n.text()).unwrap_or_default(),
            })
        }).collect();

        let follows: Vec<SpiderRequest> = resp.css(".next a").first()
            .and_then(|a| a.attr("href"))
            .and_then(|href| resp.follow(&href))
            .map(|r| vec![r])
            .unwrap_or_default();

        (items, follows)
    })
    .build();

let engine = Engine::infra().max_pages(10).build()?;
let (stats, _items) = engine.run(spider).await?;

println!("{}", stats.summary());
```

### Multi-callback Pipeline

Single Spider with multiple handlers for list → detail → content flows:

```rust
let spider = SpiderBuilder::new("pipeline")
    .start_urls(vec!["https://example.com/list"])
    .on("default", |resp| async move {
        let follows: Vec<_> = resp.css(".item a").iter()
            .filter_map(|a| resp.follow_with(a.attr("href").unwrap_or(""), "detail"))
            .collect();
        (vec![], follows)
    })
    .on("detail", |resp| async move {
        let follows: Vec<_> = resp.css("article a").iter()
            .filter_map(|a| resp.follow_with(a.attr("href").unwrap_or(""), "content"))
            .collect();
        (vec![], follows)
    })
    .on("content", |resp| async move {
        (vec![json!({"title": resp.css("h1").text()})], vec![])
    })
    .build();

let engine = Engine::infra().max_pages(1000).build()?;
let (stats, items) = engine.run(spider).await?;
```

## Architecture

```
src/
  fetcher/          Unified entry point (Fetcher + Response + Session)
    mod.rs          Fetcher struct, FetchMode enum, FetcherBuilder
    response.rs     Unified Response/Request types
    session.rs      Session with cookie persistence
  http/             HTTP client with TLS fingerprint emulation
  parser/           HTML parsing (CSS/XPath/adaptive/text search)
  crawl/            Spider engine (Spider trait, SpiderBuilder, Engine)
    builder.rs      ClosureSpider + SpiderBuilder::on(label, handler)
    engine.rs       Engine run loop (demand-driven unfold + buffer_unordered)
    stop.rs         StopCondition (MaxPages/MaxItems/MaxErrors/Timeout)
    scheduler.rs    Priority scheduler + dedup
    request_cache.rs  Request-level cache (from_cache flag)
  browser/          CDP browser automation (internal)
  stealth/          Anti-detection + Cloudflare bypass (internal)
    challenge.rs    JS Challenge / Managed Challenge solver
    turnstile.rs    Cloudflare Turnstile solver
    human.rs        Human behavior simulation
  proxy.rs          Proxy pool with rotation strategies
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

# Callback routing & Engine::infra() (new API)
cargo test --test callback_routing_test --test engine_infra_test --test sitemap_test

# Stop conditions & multi-spider routing
cargo test --test stop_condition_test --test multi_spider_test

# Real network tests (requires internet, uses proxy 127.0.0.1:7897)
cargo test --test cf_bypass_real_test -- --ignored
cargo test --test real_scrape_test -- --ignored
```

## Requirements

- Rust 1.80+ (edition 2021)
- Chrome/Chromium (for Dynamic/Stealth modes)
- Network access (for real tests; proxy `127.0.0.1:7897` supported)

## License

Apache-2.0
