# MCP tools use a typed gateway

Status: accepted

All MCP tool calls go through call_tool with ToolContext; tools receive typed arguments and return typed results. This removes per-tool JSON parsing, shared resource selection, and output shaping, and fixes fetch_page/adaptive_scrape bypassing the shared FetchClient.
