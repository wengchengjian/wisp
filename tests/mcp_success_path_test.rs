#![cfg(feature = "mcp")]

mod common;

use serde_json::json;
use std::sync::Arc;
use wisp::crawl::Engine;
use wisp::mcp::tools::{call_tool, ToolContext};
use wisp::storage::Store;
use wisp::{FetchClient, FetchClientConfig, MemoryStore};

async fn tool_ctx<'a>(
    store: &'a Arc<dyn Store>,
    engine: &'a Engine,
    fetch_client: &'a Arc<FetchClient>,
) -> ToolContext<'a> {
    ToolContext {
        store,
        engine,
        fetch_client,
    }
}

#[tokio::test]
async fn fetch_page_success_returns_html() {
    let base = common::spawn_localhost_html_server("<h1>MCP OK</h1>").await;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let engine = Engine::infra()
        .fetch_client_config(FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        })
        .build()
        .unwrap();
    let client = Arc::new(
        FetchClient::new(FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        })
        .unwrap(),
    );
    let ctx = tool_ctx(&store, &engine, &client).await;

    let result = call_tool("fetch_page", json!({ "url": base }), &ctx)
        .await
        .expect("fetch_page should succeed");
    assert_eq!(result["status"], 200);
    assert!(result["html"].as_str().unwrap().contains("MCP OK"));
}

#[tokio::test]
async fn adaptive_scrape_success_and_relocation() {
    let base = common::spawn_localhost_mutable_html_server(
        r#"<div class="first"><span>Target Value</span></div>"#,
        r#"<div class="relocated"><span>Target Value</span></div>"#,
    )
    .await;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let engine = Engine::infra()
        .fetch_client_config(FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        })
        .build()
        .unwrap();
    let client = Arc::new(
        FetchClient::new(FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        })
        .unwrap(),
    );
    let ctx = tool_ctx(&store, &engine, &client).await;
    let args = json!({
        "url": base,
        "selector": ".first",
        "key": "target",
    });

    let first = call_tool("adaptive_scrape", args.clone(), &ctx)
        .await
        .expect("first adaptive_scrape should succeed");
    assert_eq!(first["found"], true);

    let relocated = call_tool("adaptive_scrape", args, &ctx)
        .await
        .expect("relocation adaptive_scrape should succeed");
    assert_eq!(relocated["found"], true);
    assert_eq!(relocated["text"].as_str().unwrap().trim(), "Target Value");
}
