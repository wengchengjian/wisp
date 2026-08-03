//! MCP tool shared context.

use std::sync::Arc;

use wisp_crawl::Engine;
use wisp_fetcher::FetchClient;
use wisp_storage::Store;

/// Shared resources available to every MCP tool.
pub struct ToolContext<'a> {
    /// Persistence store for checkpoint/adaptive snapshot tools.
    pub store: &'a Arc<dyn Store>,
    /// Shared crawl Engine.
    pub engine: &'a Engine,
    /// Shared FetchClient for HTTP/browser/stealth transports.
    pub fetch_client: &'a Arc<FetchClient>,
}
