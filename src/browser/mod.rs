pub mod launch;

use std::sync::Arc;
use tokio::process::{Child, Command};
use std::process::Stdio;

use crate::cdp::pipe::PipeTransport;
use crate::cdp::session::CdpSession;
use crate::config::LaunchOptions;
use crate::error::{PatchrightError, Result};
use crate::page::Page;

pub struct Browser {
    session: Arc<CdpSession>,
    process: Child,
    #[allow(dead_code)]
    options: LaunchOptions,
}

impl Browser {
    pub async fn launch(options: LaunchOptions) -> Result<Self> {
        let executable = launch::resolve_executable(&options)?;
        let mut args = launch::build_stealth_args(&options);
        // Add remote-debugging-pipe (build_stealth_args returns without -- prefix)
        args.push("remote-debugging-pipe".to_string());

        // Prepend -- to all args for Chrome command line
        let chrome_args: Vec<String> = args.iter()
            .map(|a| format!("--{a}"))
            .collect();

        let mut child = Command::new(&executable)
            .args(&chrome_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PatchrightError::LaunchFailed(e.to_string()))?;

        let stdin = child.stdin.take()
            .ok_or_else(|| PatchrightError::LaunchFailed("failed to get stdin".into()))?;
        let stdout = child.stdout.take()
            .ok_or_else(|| PatchrightError::LaunchFailed("failed to get stdout".into()))?;

        let transport = PipeTransport::new(stdin, stdout);
        let session = CdpSession::new(transport);
        session.spawn_reader();

        // Wait a moment for Chrome to initialize
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        Ok(Self { session, process: child, options })
    }

    pub async fn new_page(&self) -> Result<Page> {
        Page::create(Arc::clone(&self.session)).await
    }

    pub async fn close(mut self) -> Result<()> {
        let _ = self.session.execute("Browser.close", serde_json::json!({})).await;
        let _ = self.process.wait().await;
        Ok(())
    }
}
