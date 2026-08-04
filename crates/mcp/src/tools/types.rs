//! MCP tool shared context.

/// MCP 工具共享上下文：即 `crawl::scenario::ScenarioContext` 的薄别名。
///
/// 所有资源访问统一经 `crawl::scenario`（ADR-0018），mcp 不直接依赖底层 crate。
pub type ToolContext<'a> = wisp_crawl::scenario::ScenarioContext<'a>;
