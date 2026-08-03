use std::sync::Arc;

use super::McpCmd;

pub(super) async fn run_mcp(cmd: McpCmd) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        McpCmd::Serve {
            db,
            headless,
            human_mode,
        } => {
            let store = wisp::storage::open_store(&db)?;
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
