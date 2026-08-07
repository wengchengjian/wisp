#![cfg(feature = "mcp")]

mod common;

use serde_json::json;
use std::sync::Arc;
use wisp::crawl::Engine;
use wisp::mcp::tools::{ToolContext, call_tool};
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
