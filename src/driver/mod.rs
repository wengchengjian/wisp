pub mod protocol;

use std::process::{Command, Stdio, Child};
use std::io::{BufRead, BufReader};
use serde_json::{json, Value};
use crate::error::{PatchrightError, Result};
use protocol::PlaywrightConnection;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub struct Driver {
    pub conn: PlaywrightConnection,
    process: Child,
    #[allow(dead_code)]
    reader_handle: tokio::task::JoinHandle<()>,
    /// The root Playwright object guid
    playwright_guid: Option<String>,
    /// The chromium BrowserType guid
    chromium_guid: Option<String>,
    /// Pre-launched browser guid (if any)
    prelaunched_browser: Option<String>,
}

impl Driver {
    /// Launch the patchright driver and connect via WebSocket
    pub async fn launch() -> Result<Self> {
        Self::launch_with_options(false, "chrome").await
    }

    /// Launch the patchright driver with a specific browser
    pub async fn launch_with_browser(_browser: &str) -> Result<Self> {
        Self::launch_with_options(false, "chrome").await
    }

    /// Launch the patchright driver with full options
    pub async fn launch_with_options(headless: bool, channel: &str) -> Result<Self> {
        // Find the patchright-core cli.js path
        let cli_path = Self::find_cli()?;

        // Launch: node cli.js run-server --port=0
        let mut cmd = Command::new("node");
        cmd.arg(&cli_path)
            .arg("run-server")
            .arg("--port=0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        // On Windows, Rust auto-adds CREATE_NO_WINDOW when all stdio are non-inherit.
        // For headed mode, we must NOT suppress the window or Chrome won't show UI.
        #[cfg(windows)]
        if headless {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        } else {
            // CREATE_NEW_CONSOLE allows Chrome to create its browser window
            cmd.creation_flags(0x00000010); // CREATE_NEW_CONSOLE
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| PatchrightError::LaunchFailed(format!("spawn driver: {e}")))?;

        // Read the WebSocket URL from stdout
        let stdout = child.stdout.take()
            .ok_or_else(|| PatchrightError::LaunchFailed("no stdout".into()))?;

        let ws_url = Self::read_ws_url(stdout)?;
        tracing::info!("Driver WebSocket URL: {}", ws_url);

        // Connect with browser parameters for pre-launched browser
        let ws_url = if ws_url.ends_with('/') {
            format!("{}?browser=chromium&headless={}&channel={}", ws_url, headless, channel)
        } else {
            format!("{}/?browser=chromium&headless={}&channel={}", ws_url, headless, channel)
        };
        tracing::info!("Connecting to: {}", ws_url);

        // Connect via WebSocket
        let (conn, reader_handle) = PlaywrightConnection::connect(&ws_url).await?;

        Ok(Self { conn, process: child, reader_handle, playwright_guid: None, chromium_guid: None, prelaunched_browser: None })
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

    /// Initialize the Playwright connection.
    /// Sends the initialize command and waits for the response.
    pub async fn initialize(&mut self) -> Result<String> {
        // Send initialize command
        let result = self.conn.send_command("", "initialize", json!({
            "sdkLanguage": "javascript"
        })).await?;

        tracing::info!("Initialize response: {:?}", result);

        // Get the Playwright guid from the response
        let playwright_guid = result.get("playwright")
            .and_then(|p| p.get("guid"))
            .and_then(|g| g.as_str())
            .ok_or_else(|| PatchrightError::CdpError("no playwright guid in response".into()))?
            .to_string();

        tracing::info!("Playwright guid: {}", playwright_guid);

        // Check for pre-launched browser and chromium guid in the __create__ events
        let event = self.conn.wait_for_event(|e| {
            e.method == "__create__"
                && e.params.get("type").and_then(|t| t.as_str()) == Some("Playwright")
        }, 5000).await;

        if let Ok(event) = event {
            // Store chromium BrowserType guid
            if let Some(chromium) = event.params
                .get("initializer")
                .and_then(|i| i.get("chromium"))
                .and_then(|c| c.get("guid"))
                .and_then(|g| g.as_str())
            {
                tracing::info!("Chromium BrowserType guid: {}", chromium);
                self.chromium_guid = Some(chromium.to_string());
            }
            // Check for pre-launched browser
            if let Some(pre_browser) = event.params
                .get("initializer")
                .and_then(|i| i.get("preLaunchedBrowser"))
                .and_then(|p| p.get("guid"))
                .and_then(|g| g.as_str())
            {
                tracing::info!("Pre-launched browser: {}", pre_browser);
                self.prelaunched_browser = Some(pre_browser.to_string());
            }
        }

        self.playwright_guid = Some(playwright_guid.clone());
        Ok(playwright_guid)
    }

    /// Get the Playwright guid (must call initialize first)
    pub fn playwright_guid(&self) -> Result<&str> {
        self.playwright_guid.as_deref()
            .ok_or_else(|| PatchrightError::CdpError("not initialized, call initialize() first".into()))
    }

    /// Get the pre-launched browser guid (if any)
    pub fn prelaunched_browser(&self) -> Option<&str> {
        self.prelaunched_browser.as_deref()
    }

    /// Debug: get chromium guid
    pub fn chromium_guid_debug(&self) -> Option<&str> {
        self.chromium_guid.as_deref()
    }

    /// Launch a browser (chromium with chrome channel)
    pub async fn launch_browser(&self, headless: bool, channel: &str) -> Result<String> {
        let chromium_guid = self.chromium_guid.as_deref()
            .ok_or_else(|| PatchrightError::CdpError("no chromium guid, call initialize() first".into()))?;

        tracing::info!("Launching browser via BrowserType: {}", chromium_guid);

        // Launch browser using the BrowserType
        let result = self.conn.send_command(chromium_guid, "launch", json!({
            "headless": headless,
            "channel": channel,
            "timeout": 30000.0,
            "handleSIGINT": false,
            "handleSIGTERM": false,
            "handleSIGHUP": false
        })).await?;

        let browser_guid = result.get("browser")
            .and_then(|b| b.get("guid"))
            .and_then(|g| g.as_str())
            .ok_or_else(|| PatchrightError::CdpError("no browser guid in launch response".into()))?
            .to_string();

        tracing::info!("Browser launched: {}", browser_guid);
        Ok(browser_guid)
    }

    /// Create a new browser context
    pub async fn new_context(&self, browser_guid: &str) -> Result<String> {
        let result = self.conn.send_command(browser_guid, "newContext", json!({})).await?;

        let context_guid = result.get("context")
            .and_then(|c| c.get("guid"))
            .and_then(|g| g.as_str())
            .ok_or_else(|| PatchrightError::CdpError("no context guid in response".into()))?
            .to_string();

        tracing::info!("Context created: {}", context_guid);
        Ok(context_guid)
    }

    /// Create a new page in a context
    /// Returns (page_guid, main_frame_guid)
    pub async fn new_page(&self, context_guid: &str) -> Result<(String, String)> {
        let result = self.conn.send_command(context_guid, "newPage", json!({})).await?;

        let page_guid = result.get("page")
            .and_then(|p| p.get("guid"))
            .and_then(|g| g.as_str())
            .ok_or_else(|| PatchrightError::CdpError("no page guid in response".into()))?
            .to_string();

        // Get the main frame guid from the __create__ event for Page
        let event = self.conn.wait_for_event(|e| {
            e.method == "__create__"
                && e.params.get("type").and_then(|t| t.as_str()) == Some("Page")
                && e.params.get("guid").and_then(|g| g.as_str()) == Some(page_guid.as_str())
        }, 5000).await?;

        let frame_guid = event.params
            .get("initializer")
            .and_then(|i| i.get("mainFrame"))
            .and_then(|f| f.get("guid"))
            .and_then(|g| g.as_str())
            .ok_or_else(|| PatchrightError::CdpError("no mainFrame guid in Page initializer".into()))?
            .to_string();

        tracing::info!("Page created: {} (main frame: {})", page_guid, frame_guid);
        Ok((page_guid, frame_guid))
    }

    /// Navigate to a URL (uses the main frame)
    pub async fn goto(&self, frame_guid: &str, url: &str) -> Result<Value> {
        let result = self.conn.send_command(frame_guid, "goto", json!({
            "url": url,
            "timeout": 30000
        })).await?;
        tracing::info!("Navigated to: {}", url);
        Ok(result)
    }

    /// Evaluate a JavaScript expression on a frame
    pub async fn evaluate(&self, frame_guid: &str, expression: &str) -> Result<Value> {
        let result = self.conn.send_command(frame_guid, "evaluateExpression", json!({
            "expression": expression,
            "isFunction": false,
            "arg": {
                "value": {"v": "undefined"},
                "handles": []
            }
        })).await?;
        Ok(result)
    }

    /// Take a screenshot (returns base64-encoded PNG)
    pub async fn screenshot(&self, page_guid: &str) -> Result<String> {
        let result = self.conn.send_command(page_guid, "screenshot", json!({
            "type": "png",
            "timeout": 30000
        })).await?;

        let binary = result.get("binary")
            .and_then(|b| b.as_str())
            .ok_or_else(|| PatchrightError::CdpError("no binary in screenshot response".into()))?
            .to_string();

        Ok(binary)
    }

    /// Close a browser
    pub async fn close_browser(&self, browser_guid: &str) -> Result<()> {
        self.conn.send_command(browser_guid, "close", json!({})).await?;
        tracing::info!("Browser closed");
        Ok(())
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
