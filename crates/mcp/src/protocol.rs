//! MCP 工具定义与输入参数 JSON Schema。

use serde_json::json;
use std::sync::LazyLock;

/// MCP 工具定义
pub struct Tool {
    /// 工具名称。
    pub name: &'static str,
    /// 工具描述。
    pub description: &'static str,
    /// 输入参数 JSON Schema。
    pub input_schema: serde_json::Value,
}

/// 5 个工具覆盖核心场景
// 注：计划原写 `pub const TOOLS: &[Tool]`，但 serde_json::json! 宏非 const fn，
// 无法在 const 上下文求值。改用 std::sync::LazyLock（Rust 1.80+ 稳定）。
pub static TOOLS: LazyLock<Vec<Tool>> = LazyLock::new(|| {
    vec![
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
            description: "爬取站点，返回 JSONL。用内置 SpiderBuilder 按 CSS 选择器提取。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "start_urls": { "type": "array", "items": { "type": "string" } },
                    "css_selector": { "type": "string", "description": "每页提取的 CSS 选择器" },
                    "max_pages": { "type": "integer", "default": 100 },
                    "follow_pattern": { "type": "string", "description": "可选：仅跟随匹配此正则的链接" },
                    "max_depth": { "type": "integer", "default": 0, "description": "最大跟随深度，0 表示不限制" },
                    "allowed_domains": { "type": "array", "items": { "type": "string" }, "description": "可选：仅跟随这些域名的链接" }
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
        #[cfg(feature = "stealth")]
        Tool {
            name: "stealth_fetch",
            description: "浏览器隐身抓取（CF 挑战解决 + 人类行为模拟，复用共享浏览器池）。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string" }
                },
                "required": ["url"]
            }),
        },
    ]
});
