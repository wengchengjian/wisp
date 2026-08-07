use super::server::{ServerContext, handle_initialize, handle_tools_call, handle_tools_list};
use crate::tools::ToolContext;
use serde_json::json;
use std::sync::Arc;
use wisp_core::error::{McpError, WispError};
use wisp_crawl::Engine;
use wisp_fetcher::{FetchClient, FetchClientConfig};
use wisp_storage::Store;

fn test_fetch_client() -> Arc<FetchClient> {
    Arc::new(
        FetchClient::new(FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        })
        .expect("build test fetch client"),
    )
}

fn test_context<'a>(
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

#[test]
fn test_tools_list_has_expected_tools() {
    let list = handle_tools_list();
    let tools = list.get("tools").unwrap().as_array().unwrap();
    #[cfg(feature = "stealth")]
    assert_eq!(tools.len(), 4, "stealth feature 下应有 4 个工具");
    #[cfg(not(feature = "stealth"))]
    assert_eq!(tools.len(), 3, "无 stealth feature 时应为 3 个工具");
    let names: Vec<&str> = tools
        .iter()
        .map(|t| t.get("name").unwrap().as_str().unwrap())
        .collect();
    assert!(names.contains(&"fetch_page"));
    assert!(names.contains(&"extract_css"));
    assert!(names.contains(&"crawl_site"));
    #[cfg(feature = "stealth")]
    assert!(names.contains(&"stealth_fetch"));
}

#[test]
fn test_handle_initialize() {
    let init = handle_initialize();
    assert_eq!(init["serverInfo"]["name"], "wisp");
    assert!(init["capabilities"]["tools"].is_object());
}

#[tokio::test]
async fn test_handle_tools_call_unknown_tool() {
    let store: Arc<dyn Store> = Arc::new(wisp_storage::MemoryStore::default());
    let ctx = ServerContext::new(store, test_fetch_client()).unwrap();
    let req = json!({
        "params": { "name": "nonexistent", "arguments": {} }
    });
    let result = handle_tools_call(req, &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        WispError::Mcp(McpError::UnknownTool(n)) => assert_eq!(n, "nonexistent"),
        other => panic!("预期 McpUnknownTool, 得到 {:?}", other),
    }
}

#[cfg(feature = "stealth")]
#[tokio::test]
async fn stealth_fetch_requires_url() {
    let client = test_fetch_client();
    let store: Arc<dyn Store> = Arc::new(wisp_storage::MemoryStore::default());
    let engine = Engine::infra()
        .max_pages(100)
        .obey_robots(false)
        .build()
        .unwrap();
    let ctx = test_context(&store, &engine, &client);
    let result = crate::tools::call_tool("stealth_fetch", json!({}), &ctx).await;
    let err = result.expect_err("缺少 url 应报错");
    assert!(err.to_string().contains("url"), "错误应说明缺少 url: {err}");
}

#[tokio::test]
async fn fetch_page_rejects_private_ip() {
    let store: Arc<dyn Store> = Arc::new(wisp_storage::MemoryStore::default());
    let engine = Engine::infra()
        .max_pages(100)
        .obey_robots(false)
        .build()
        .unwrap();
    let client = test_fetch_client();
    let ctx = test_context(&store, &engine, &client);
    let result =
        crate::tools::call_tool("fetch_page", json!({ "url": "http://127.0.0.1/" }), &ctx).await;
    let err = result.expect_err("MCP fetch_page 应拒绝内网地址");
    assert!(
        err.to_string().contains("拒绝"),
        "错误应来自 SSRF 校验: {err}"
    );
}
