//! MCP adaptive_scrape 工具。

use serde_json::{json, Value};
#[cfg(not(feature = "sqlite"))]
use std::path::PathBuf;
use std::sync::Arc;
use wisp_core::error::{McpError, Result, WispError};
use wisp_http::Client;
use wisp_parser::Node;
#[cfg(feature = "sqlite")]
use wisp_storage::SqliteStore;
use wisp_storage::{FileStore, Store};

#[cfg(feature = "sqlite")]
fn store_from_db_path(db_path: &str) -> Result<Arc<dyn Store>> {
    if db_path != ":memory:" {
        return Ok(Arc::new(SqliteStore::open(std::path::Path::new(db_path))?));
    }
    Ok(Arc::new(FileStore::default()))
}

#[cfg(not(feature = "sqlite"))]
fn store_from_db_path(db_path: &str) -> Result<Arc<dyn Store>> {
    if db_path == ":memory:" {
        return Ok(Arc::new(FileStore::default()));
    }
    Ok(Arc::new(FileStore::with_dir(PathBuf::from(db_path))))
}

/// 自适应抓取：CSS 失败时用快照存储重定位。
async fn adaptive_scrape_impl(
    url: &str,
    selector: &str,
    key: &str,
    store: &Arc<dyn Store>,
) -> Result<Value> {
    wisp_core::utils::validate_url(url)?;
    let client = Client::builder().build()?;
    let resp = client.get(url, &[]).await?;
    let html = resp.text()?;
    let doc = Node::from_html(&html);
    use wisp_crawl::AdaptiveTracker;
    let tracker = AdaptiveTracker::new(Arc::clone(store));
    let found = tracker
        .css_adaptive(
            &doc,
            selector,
            key,
            url,
            true,
            wisp_parser::DEFAULT_TOLERANCE,
        )
        .await?;
    match found {
        Some(node) => Ok(json!({
            "url": url,
            "found": true,
            "text": node.text(),
            "html": node.html()
        })),
        None => Ok(json!({ "url": url, "found": false })),
    }
}

pub async fn adaptive_scrape(args: Value, store: &Arc<dyn Store>) -> Result<Value> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'url'".into())))?;
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'selector'".into())))?;
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'key'".into())))?;
    let db_path = args
        .get("db_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let effective_store: Arc<dyn Store> = match db_path {
        Some(path) => store_from_db_path(path)?,
        None => Arc::clone(store),
    };
    adaptive_scrape_impl(url, selector, key, &effective_store).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_path_creates_expected_backend() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshots.db");
        let _store = store_from_db_path(path.to_str().unwrap()).unwrap();
        #[cfg(feature = "sqlite")]
        assert!(path.is_file(), "sqlite 应创建 db 文件");
        #[cfg(not(feature = "sqlite"))]
        assert!(path.is_dir(), "无 sqlite 应创建 FileStore 目录");
    }
}
