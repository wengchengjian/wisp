use std::sync::Arc;

use super::McpCmd;

pub(super) async fn run_mcp(cmd: McpCmd) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        McpCmd::Serve {
            db,
            headless,
            human_mode,
        } => {
            let store = build_store(&db)?;
            let fetch_client = Arc::new(wisp::FetchClient::new(wisp::FetchClientConfig {
                headless,
                human_mode,
                ..Default::default()
            })?);
            wisp::mcp::serve(store, fetch_client).await?;
        }
    }
    Ok(())
}

fn build_store(db: &str) -> Result<Arc<dyn wisp::Store>, Box<dyn std::error::Error>> {
    #[cfg(feature = "sqlite")]
    {
        if db != ":memory:" && !db.is_empty() {
            Ok(Arc::new(wisp::SqliteStore::open(std::path::Path::new(db))?))
        } else {
            Ok(Arc::new(wisp::FileStore::default()))
        }
    }
    #[cfg(not(feature = "sqlite"))]
    {
        if db.is_empty() || db == ":memory:" {
            return Ok(Arc::new(wisp::FileStore::default()));
        }
        eprintln!("当前构建未启用 sqlite，使用 FileStore 目录: {db}");
        Ok(Arc::new(wisp::FileStore::with_dir(
            std::path::PathBuf::from(db),
        )))
    }
}
