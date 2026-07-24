//! MCP (Model Context Protocol) server over stdio JSON-RPC 2.0.
//!
//! 工具定义在 TOOLS 常量，实现 in tools.rs.

pub mod tools;

use serde_json::{Value, json};
use std::sync::{Arc, LazyLock};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::error::{WispError, Result};
use crate::storage::Store;
use crate::crawl::Engine;

/// MCP 工具定义
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

/// 6 个工具覆盖核心场景
// 注：计划原写 `pub const TOOLS: &[Tool]`，但 serde_json::json! 宏非 const fn，
// 无法在 const 上下文求值。改用 std::sync::LazyLock（Rust 1.80+ 稳定）。
pub static TOOLS: LazyLock<Vec<Tool>> = LazyLock::new(|| vec![
    Tool {
        name: "fetch_page",
        description: "抓取单个网页，返回 HTML 文本。支持 wreq TLS 指纹模拟绕过轻度反 bot。",
        input_schema: json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "目标 URL" },
                "emulation": {
                    "type": "string",
                    "enum": ["chrome", "firefox", "safari"],
                    "description": "浏览器指纹模拟，默认 chrome"
                }
            },
            "required": ["url"]
        }),
    },
    Tool {
        name: "extract_css",
        description: "用 CSS 选择器从 HTML 提取元素，返回文本/属性列表。",
        input_schema: json!({
            "type": "object",
            "properties": {
                "html": { "type": "string", "description": "HTML 文本" },
                "selector": { "type": "string", "description": "CSS 选择器" },
                "attr": { "type": "string", "description": "可选：提取该属性而非文本" }
            },
            "required": ["html", "selector"]
        }),
    },
    Tool {
        name: "crawl_site",
        description: "爬取站点，返回 JSONL。用内置 SimpleSpider 按 CSS 选择器提取。",
        input_schema: json!({
            "type": "object",
            "properties": {
                "start_urls": { "type": "array", "items": { "type": "string" } },
                "css_selector": { "type": "string", "description": "每页提取的 CSS 选择器" },
                "max_pages": { "type": "integer", "default": 100 },
                "follow_pattern": { "type": "string", "description": "可选：跟随链接的正则" }
            },
            "required": ["start_urls", "css_selector"]
        }),
    },
    Tool {
        name: "adaptive_scrape",
        description: "自适应抓取：CSS 失败时用 SQLite 快照重定位元素（长期监控）。",
        input_schema: json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "selector": { "type": "string" },
                "key": { "type": "string", "description": "元素稳定标识" },
                "db_path": { "type": "string", "default": "./wisp.db" }
            },
            "required": ["url", "selector", "key"]
        }),
    },
    Tool {
        name: "stealth_fetch",
        description: "浏览器模式抓取（绕 CF Turnstile 等重度反 bot）。",
        input_schema: json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "headless": { "type": "boolean", "default": true },
                "human_mode": { "type": "boolean", "default": false, "description": "启用人类行为模拟" }
            },
            "required": ["url"]
        }),
    },
]);

/// MCP server 主循环（stdio JSON-RPC 2.0）
pub async fn serve(store: Arc<Store>) -> Result<()> {
    // 启动时创建一个长驻共享 Engine，所有 crawl_site 调用复用
    // （共享 HTTP 连接池 / 请求缓存 / 代理池）。Engine 自身的 max_pages 作为全局兜底，
    // 每次 crawl_site 的 per-call 上限由 SimpleSpider 的 until() 终止策略控制。
    let engine = Engine::infra()
        .max_pages(100000)
        .build()?;

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = request.get("id").cloned();

        let response: Value = match method {
            "initialize" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": handle_initialize()
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": handle_tools_list()
            }),
            "tools/call" => match handle_tools_call(request, &store, &engine).await {
                Ok(result) => json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": result
                }),
                Err(e) => json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {
                        "code": -32603,
                        "message": e.to_string()
                    }
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
                "error": {
                    "code": -32601,
                    "message": format!("unknown method: {}", method)
                }
            }),
        };

        let response_str = serde_json::to_string(&response)
            .map_err(|e| WispError::Serialize(e.to_string()))?;
        stdout.write_all(response_str.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    Ok(())
}

fn handle_initialize() -> Value {
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

fn handle_tools_list() -> Value {
    let tools: Vec<Value> = TOOLS.iter().map(|t| json!({
        "name": t.name,
        "description": t.description,
        "inputSchema": t.input_schema,
    })).collect();
    json!({"tools": tools})
}

async fn handle_tools_call(request: Value, store: &Arc<Store>, engine: &Engine) -> Result<Value> {
    let params = request.get("params")
        .ok_or_else(|| WispError::McpError("missing params".into()))?;
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let result = match name {
        "fetch_page" => tools::fetch_page(args).await,
        "extract_css" => tools::extract_css(args).await,
        "crawl_site" => tools::crawl_site(args, engine).await,
        "adaptive_scrape" => tools::adaptive_scrape(args, store).await,
        "stealth_fetch" => tools::stealth_fetch(args).await,
        _ => Err(WispError::McpUnknownTool(name.into())),
    }?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result)
                .map_err(|e| WispError::Serialize(e.to_string()))?
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tools_list_has_six_tools() {
        let list = handle_tools_list();
        let tools = list.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 5, "应有 5 个工具");
        let names: Vec<&str> = tools.iter()
            .map(|t| t.get("name").unwrap().as_str().unwrap())
            .collect();
        assert!(names.contains(&"fetch_page"));
        assert!(names.contains(&"extract_css"));
        assert!(names.contains(&"crawl_site"));
        assert!(names.contains(&"adaptive_scrape"));
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
        let store = Arc::new(Store::open_in_memory().unwrap());
        let engine = Engine::infra()
            .max_pages(100)
            .build()
            .unwrap();
        let req = json!({
            "params": { "name": "nonexistent", "arguments": {} }
        });
        let result = handle_tools_call(req, &store, &engine).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            WispError::McpUnknownTool(n) => assert_eq!(n, "nonexistent"),
            other => panic!("预期 McpUnknownTool, 得到 {:?}", other),
        }
    }
}
