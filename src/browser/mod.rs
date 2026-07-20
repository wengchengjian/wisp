pub mod launch;

use std::sync::Arc;
use std::process::Stdio;

use crate::cdp::pipe::PipeTransport;
use crate::cdp::session::CdpSession;
use crate::config::LaunchOptions;
use crate::error::{PatchrightError, Result};
use crate::page::Page;

/// The Node.js helper script that spawns Chrome with proper pipe handles.
const HELPER_SCRIPT: &str = include_str!("../helper/patchright-helper.js");

pub struct Browser {
    pub session: Arc<CdpSession>,
    process: std::process::Child,
    #[allow(dead_code)]
    options: LaunchOptions,
}

impl Browser {
    pub async fn launch(options: LaunchOptions) -> Result<Self> {
        let executable = launch::resolve_executable(&options)?;
        let mut args = launch::build_stealth_args(&options);
        args.push("remote-debugging-pipe".to_string());
        if options.headless {
            args.push("headless=new".to_string());
        }

        let chrome_args: Vec<String> = args.iter()
            .map(|a| format!("--{a}"))
            .collect();

        // Write helper script to temp file
        let helper_path = std::env::temp_dir().join("patchright-helper.js");
        std::fs::write(&helper_path, HELPER_SCRIPT)
            .map_err(|e| PatchrightError::LaunchFailed(format!("write helper: {e}")))?;

        // Find node executable
        let node = which::which("node")
            .map_err(|_| PatchrightError::LaunchFailed("Node.js not found. Install Node.js for pipe-based CDP.".into()))?;

        // Spawn helper using std::process::Command (tokio's version causes pipe issues on Windows)
        let mut cmd_args = vec![helper_path.to_string_lossy().to_string()];
        cmd_args.push(executable.to_string_lossy().to_string());
        cmd_args.extend(chrome_args);

        let mut child = std::process::Command::new(&node)
            .args(&cmd_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| PatchrightError::LaunchFailed(e.to_string()))?;

        let std_stdin = child.stdin.take()
            .ok_or_else(|| PatchrightError::LaunchFailed("failed to get stdin".into()))?;
        let std_stdout = child.stdout.take()
            .ok_or_else(|| PatchrightError::LaunchFailed("failed to get stdout".into()))?;

        let (transport, msg_rx) = PipeTransport::new(std_stdin, std_stdout);
        let session = CdpSession::new(transport, msg_rx);
        session.spawn_reader();

        // Wait for Chrome to initialize via helper
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

        Ok(Self { session, process: child, options })
    }

    pub async fn new_page(&self) -> Result<Page> {
        Page::create(Arc::clone(&self.session)).await
    }

    pub async fn close(mut self) -> Result<()> {
        let _ = self.session.execute("Browser.close", serde_json::json!({})).await;
        let _ = self.process.kill();
        let _ = self.process.wait();
        Ok(())
    }
}
