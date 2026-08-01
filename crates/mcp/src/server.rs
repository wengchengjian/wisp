//! stdio JSON-RPC 2.0 服务循环与请求处理。

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::protocol::TOOLS;
use crate::tools;
use wisp_core::error::{McpError, ParseError, Result, WispError};
use wisp_crawl::Engine;
use wisp_storage::Store;

async fn write_json_line(stdout: &mut tokio::io::Stdout, value: &Value) -> Result<()> {
    let s = serde_json::to_string(value)
        .map_err(|e| WispError::Parse(ParseError::Serialize(e.to_string())))?;
    stdout.write_all(s.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}

async fn write_parse_error(stdout: &mut tokio::io::Stdout, error: &str) -> Result<()> {
    write_json_line(
        stdout,
        &json!({
            "jsonrpc": "2.0", "id": null,
            "error": { "code": -32700, "message": error }
        }),
    )
    .await
}

async fn dispatch_request(request: Value, store: &Arc<dyn Store>, engine: &Engine) -> Value {
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = request.get("id").cloned();
    match method {
        "initialize" => json!({
            "jsonrpc": "2.0", "id": id,
            "result": handle_initialize()
        }),
        "tools/list" => json!({
            "jsonrpc": "2.0", "id": id,
            "result": handle_tools_list()
        }),
        "tools/call" => match handle_tools_call(request, store, engine).await {
            Ok(result) => json!({
                "jsonrpc": "2.0", "id": id,
                "result": result
            }),
            Err(e) => json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32603, "message": e.to_string() }
            }),
        },
        "resources/list" => json!({
            "jsonrpc": "2.0", "id": id,
            "result": {"resources": []}
        }),
        "prompts/list" => json!({
            "jsonrpc": "2.0", "id": id,
            "result": {"prompts": []}
        }),
        _ => json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32601, "message": format!("unknown method: {}", method) }
        }),
    }
}

async fn handle_request_line(
    line: &str,
    store: &Arc<dyn Store>,
    engine: &Engine,
    stdout: &mut tokio::io::Stdout,
) -> Result<()> {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return write_parse_error(stdout, &format!("Parse error: {e}")).await;
        }
    };
    let id = request.get("id").cloned();
    let is_notification = id.is_none();
    let response = dispatch_request(request, store, engine).await;
    if is_notification {
        return Ok(());
    }
    write_json_line(stdout, &response).await
}

/// MCP server 主循环（stdio JSON-RPC 2.0）
pub async fn serve(store: Arc<dyn Store>) -> Result<()> {
    let engine = Engine::infra()
        .max_pages(100000)
        .obey_robots(false)
        .build()?;

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        handle_request_line(&line, &store, &engine, &mut stdout).await?;
    }

    Ok(())
}

pub(super) fn handle_initialize() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "wisp",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

pub(super) fn handle_tools_list() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect();
    json!({ "tools": tools })
}

pub(super) async fn handle_tools_call(
    request: Value,
    store: &Arc<dyn Store>,
    engine: &Engine,
) -> Result<Value> {
    let params = request
        .get("params")
        .ok_or_else(|| WispError::Mcp(McpError::General("missing params".into())))?;
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let result = match name {
        "fetch_page" => tools::fetch_page(args).await,
        "extract_css" => tools::extract_css(args).await,
        "crawl_site" => tools::crawl_site(args, engine).await,
        "adaptive_scrape" => tools::adaptive_scrape(args, store).await,
        #[cfg(feature = "browser")]
        "stealth_fetch" => tools::stealth_fetch(args).await,
        _ => Err(WispError::Mcp(McpError::UnknownTool(name.into()))),
    }?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result)
                .map_err(|e| WispError::Parse(ParseError::Serialize(e.to_string())))?
        }]
    }))
}
