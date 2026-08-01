use super::server::{handle_initialize, handle_tools_call, handle_tools_list};
use serde_json::json;
use std::sync::Arc;
use wisp_core::error::{McpError, WispError};
use wisp_crawl::Engine;
use wisp_storage::Store;

#[test]
fn test_tools_list_has_five_tools() {
    let list = handle_tools_list();
    let tools = list.get("tools").unwrap().as_array().unwrap();
    #[cfg(feature = "browser")]
    assert_eq!(tools.len(), 5, "browser feature 下应有 5 个工具");
    #[cfg(not(feature = "browser"))]
    assert_eq!(tools.len(), 4, "无 browser feature 时应为 4 个工具");
    let names: Vec<&str> = tools
        .iter()
        .map(|t| t.get("name").unwrap().as_str().unwrap())
        .collect();
    assert!(names.contains(&"fetch_page"));
    assert!(names.contains(&"extract_css"));
    assert!(names.contains(&"crawl_site"));
    assert!(names.contains(&"adaptive_scrape"));
    #[cfg(feature = "browser")]
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
    let engine = Engine::infra()
        .max_pages(100)
        .obey_robots(false)
        .build()
        .unwrap();
    let req = json!({
        "params": { "name": "nonexistent", "arguments": {} }
    });
    let result = handle_tools_call(req, &store, &engine).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        WispError::Mcp(McpError::UnknownTool(n)) => assert_eq!(n, "nonexistent"),
        other => panic!("预期 McpUnknownTool, 得到 {:?}", other),
    }
}

#[tokio::test]
async fn fetch_page_rejects_private_ip() {
    let result = crate::tools::fetch_page(json!({ "url": "http://127.0.0.1/" })).await;
    let err = result.expect_err("MCP fetch_page 应拒绝内网地址");
    assert!(
        err.to_string().contains("拒绝"),
        "错误应来自 SSRF 校验: {err}"
    );
}

#[tokio::test]
async fn adaptive_scrape_rejects_private_ip() {
    let store: Arc<dyn Store> = Arc::new(wisp_storage::MemoryStore::default());
    let result = crate::tools::adaptive_scrape(
        json!({ "url": "http://127.0.0.1/", "selector": "p", "key": "k" }),
        &store,
    )
    .await;
    let err = result.expect_err("MCP adaptive_scrape 应拒绝内网地址");
    assert!(
        err.to_string().contains("拒绝"),
        "错误应来自 SSRF 校验: {err}"
    );
}
