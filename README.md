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
use wisp::{Fetcher, parser::ResponseExt};

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

## Spider Crawling

```rust
use wisp::{SpiderBuilder, Engine, FetchMode, parser::ResponseExt};
use wisp::crawl::Request;
use serde_json::json;

let spider = SpiderBuilder::new("quotes")
    .start_urls(vec!["https://quotes.toscrape.com/"])
    .on("default", |resp| async move {
        let items = resp.css(".quote").iter().map(|q| {
            json!({
                "text": q.select_one(".text").map(|n| n.text()).unwrap_or_default(),
                "author": q.select_one(".author").map(|n| n.text()).unwrap_or_default(),
            })
        }).collect();

        let follows: Vec<Request> = resp.css(".next a").first()
            .and_then(|a| a.attr("href"))
            .and_then(|href| resp.follow(&href))
            .map(|r| vec![r])
            .unwrap_or_default();

        (items, follows)
    })
    .build();

let engine = Engine::infra()
    .max_pages(10)
    .fetch_mode(FetchMode::Http)
    .download_delay_ms(200)
    .obey_robots(false)
    .build()?;
let (stats, _items) = engine.run(spider).await?;

println!("{}", stats.summary());
```

### Multi-callback Pipeline

Single Spider with multiple handlers for list → detail → content flows:

```rust
use wisp::{SpiderBuilder, parser::ResponseExt};

let spider = SpiderBuilder::new("pipeline")
    .start_urls(vec!["https://example.com/list"])
    .on("default", |resp| async move {
        let follows: Vec<_> = resp.css(".item a").iter()
            .filter_map(|a| resp.follow_with(&a.attr("href").unwrap_or_default(), "detail"))
            .collect();
        (vec![], follows)
    })
    .on("detail", |resp| async move {
        let follows: Vec<_> = resp.css("article a").iter()
            .filter_map(|a| resp.follow_with(&a.attr("href").unwrap_or_default(), "content"))
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

### Multi-Spider Shared Queue

每个阶段可以是一个独立 Spider，`Engine::run_many` 使用共享队列并按 callback 路由；每个 Spider 独立 `until()` 和 stats：

```rust
use wisp::crawl::stop::MaxPages;
use wisp::parser::ResponseExt;

let home = SpiderBuilder::new("home")
    .start_urls(vec!["https://example.com/list"])
    .on("default", |resp| async move {
        let follows = resp.css(".item a").iter()
            .filter_map(|a| resp.follow_with(&a.attr("href").unwrap_or_default(), "detail"))
            .collect();
        (vec![], follows)
    })
    .build();

let detail = SpiderBuilder::new("detail")
    .on("detail", |resp| async move { (vec![], vec![]) })
    .until(MaxPages(10))
    .build();

let engine = Engine::infra().max_pages(1000).build()?;
let (stats, items) = engine.run_many(vec![home, detail]).await?;
```

传输能力（headers/UA/Cookie Challenge）和共享 pipeline 配置在 `EngineBuilder` 上，不在 `SpiderBuilder`：

```rust
let engine = Engine::infra()
    .headers(vec![("Accept".into(), "text/html".into())])
    .ua_rotation(wisp::crawl::middleware::UaRotationMiddleware::desktop())
    .cookie_challenge(true)
    .pipeline(Arc::new(wisp::crawl::middleware::JsonlWriterPipeline::new("items.jsonl")))
    .build()?;
```

## Architecture

```text
Cargo.toml                       workspace + facade package（bin/example/test/bench）
src/lib.rs                       facade：模块 re-export + 顶层 API
crates/core/                     Request/Response/Method/WispError/config/text/utils/encoding
crates/storage/                  Store + Memory/File/Sqlite + CachedResponse
crates/parser/                   Node/NodeList/adaptive + ResponseExt
crates/proxy/                    ProxyPool/RotationStrategy + config_file
crates/http/                     wreq Client/Config/DomainBlocker
crates/browser/                  CDP browser automation
crates/stealth/                  Challenge solver / human behavior / Turnstile
crates/fetcher/                  Fetcher/FetchClient/FetchMode
crates/crawl/                    Engine/Spider/middleware/scheduler/runtime
crates/mcp/                      MCP server
```

依赖方向单向：`core <- storage/parser/proxy/http/browser <- stealth <- fetcher <- crawl <- mcp <- facade`。

## 存储后端

wisp 支持三种可插拔存储后端，通过 `Store` trait 抽象（仅 4 个底层 KV 原语：`set`/`get`/`delete`/`set_with_ttl`）：

| 后端 | feature 开关 | 默认用途 | 持久化 |
|------|-------------|---------|--------|
| `MemoryStore` | 始终启用 | 响应缓存（需显式 `cache_store` 开启） | 否（进程退出丢失） |
| `FileStore` | 始终启用 | 断点续爬（checkpoint_store 默认） | 是（`./wisp-data/`） |
| `SqliteStore` | `sqlite` feature | 用户显式选择 | 是（`.db` 文件） |

默认不启用 sqlite，使用 FileStore + MemoryStore 组合：

```toml
[dependencies]
wisp = { path = "../wisp" }
```

启用 sqlite：

```toml
[dependencies]
wisp = { path = "../wisp", features = ["sqlite"] }
```

业务层 API（`save_checkpoint`/`load_response`/`save_element` 等）以自由函数形式提供，调用底层 `Store` trait 原语并处理序列化。自定义存储后端：实现 `Store` trait 的 4 个原语，通过 `EngineBuilder::cache_store(...)` / `.checkpoint(...)` 注入。

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

默认测试工具为 cargo-nextest：

```bash
# 默认完整测试（快速并行执行，不含 doctest）
cargo nextest run --workspace --all-features

# doctest 单独执行
cargo test --doc
```

libtest 兼容命令仍可用：

```bash
# Unit tests (no network required)
cargo test --lib

# Builder & unified API tests
cargo test --test builder_api_test --test unified_fetcher_test

# Callback routing & Engine::infra() (new API)
cargo test --test callback_routing_test --test engine_infra_test --test sitemap_test

# Stop conditions & multi-spider routing
cargo test --test stop_condition_test --test multi_spider_test --test multi_spider_routing_test --test novel_10_books_test

# Real network/browser tests (requires internet, proxy 127.0.0.1:7897, Chrome)
cargo nextest run --test cf_bypass_real_test --test real_scrape_test --run-ignored all --no-fail-fast

# Chrome-only concurrency/cancel tests (no network required)
cargo nextest run --test browser_concurrency_cancel_test --run-ignored all
```

## Requirements

- Rust 1.91.1+ (edition 2024)
- Chrome/Chromium (for Dynamic/Stealth modes)
- Network access (for real tests; proxy `127.0.0.1:7897` supported)

## License

Apache-2.0
