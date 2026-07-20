pub mod protocol;

use std::process::{Command, Stdio, Child};
use std::io::{BufRead, BufReader};
use serde_json::Value;
use crate::error::{PatchrightError, Result};
use protocol::PlaywrightConnection;

pub struct Driver {
    pub conn: PlaywrightConnection,
    process: Child,
    #[allow(dead_code)]
    reader_handle: tokio::task::JoinHandle<()>,
}

impl Driver {
    /// Launch the patchright driver and connect via WebSocket
    pub async fn launch() -> Result<Self> {
        // Find the patchright-core cli.js path
        let cli_path = Self::find_cli()?;

        // Launch: node cli.js run-server --port=0
        let mut child = Command::new("node")
            .arg(&cli_path)
            .arg("run-server")
            .arg("--port=0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| PatchrightError::LaunchFailed(format!("spawn driver: {e}")))?;

        // Read the WebSocket URL from stdout
        let stdout = child.stdout.take()
            .ok_or_else(|| PatchrightError::LaunchFailed("no stdout".into()))?;

        let ws_url = Self::read_ws_url(stdout)?;

        // Connect via WebSocket
        let (conn, reader_handle) = PlaywrightConnection::connect(&ws_url).await?;

        Ok(Self { conn, process: child, reader_handle })
    }

    fn find_cli() -> Result<String> {
        // Check node_modules/patchright-core/cli.js
        let paths = [
            "node_modules/patchright-core/cli.js",
            "node_modules/patchright/node_modules/patchright-core/cli.js",
        ];
        for p in &paths {
            if std::path::Path::new(p).exists() {
                return Ok(p.to_string());
            }
        }
        Err(PatchrightError::LaunchFailed(
            "patchright-core not found. Run: npm install patchright".into()
        ))
    }

    fn read_ws_url(stdout: std::process::ChildStdout) -> Result<String> {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = line.map_err(|e| PatchrightError::LaunchFailed(format!("read: {e}")))?;
            // Looking for: "Listening on ws://..."
            if let Some(pos) = line.find("ws://") {
                return Ok(line[pos..].trim().to_string());
            }
        }
        Err(PatchrightError::LaunchFailed("no ws:// URL from driver".into()))
    }

    /// Initialize the Playwright connection
    pub async fn initialize(&self) -> Result<Value> {
        // The root object is "" (empty string) in Playwright protocol
        self.conn.send_command("", "__create__", serde_json::json!({
            "type": "Playwright",
            "guid": "playwright",
            "initializer": {}
        })).await
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
