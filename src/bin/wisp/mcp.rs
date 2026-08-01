use std::sync::Arc;

use super::McpCmd;

pub(super) async fn run_mcp(cmd: McpCmd) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        McpCmd::Serve { db } => {
            let store = build_store(&db)?;
            wisp::mcp::serve(store).await?;
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
        let _ = db;
        Ok(Arc::new(wisp::FileStore::default()))
    }
}
