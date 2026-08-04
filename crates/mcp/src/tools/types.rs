//! MCP tool shared context.

use std::sync::Arc;

use wisp_core::error::Result;
use wisp_crawl::Engine;
use wisp_fetcher::FetchClient;
use wisp_storage::{Store, open_store};

/// Shared resources available to every MCP tool.
pub struct ToolContext<'a> {
    /// Persistence store for checkpoint/adaptive snapshot tools.
    pub store: &'a Arc<dyn Store>,
    /// Shared crawl Engine.
    pub engine: &'a Engine,
    /// Shared FetchClient for HTTP/browser/stealth transports.
    pub fetch_client: &'a Arc<FetchClient>,
}

impl ToolContext<'_> {
    /// 获取持久化 store：优先使用指定的 `path`（非空时打开），否则回退到共享 `store`。
    ///
    /// ADR-0018：所有资源访问统一经 `ToolContext`，工具不自行 `open_store`，
    /// 保持单一资源边界。路径为空时直接返回共享 store，避免重复打开。
    pub fn get_or_open_store(&self, path: Option<&str>) -> Result<Arc<dyn Store>> {
        match path {
            Some(p) if !p.is_empty() => open_store(p),
            _ => Ok(Arc::clone(self.store)),
        }
    }
}
