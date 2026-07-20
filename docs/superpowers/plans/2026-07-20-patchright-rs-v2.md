# Patchright-RS v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace chromiumoxide WebSocket CDP with self-built pipe-based CDP client, pass Browserscan detection, and add full CLI tool.

**Architecture:** Three-phase layered refactoring: (1) pipe-based CDP transport + session management, (2) Browser/Page API rebuilt on new CDP client, (3) CLI tool with clap. All anti-detection patches preserved.

**Tech Stack:** Rust, tokio, serde/serde_json, thiserror, clap 4, reqwest, base64, zip

## Global Constraints

- NEVER send `Runtime.enable` or `Console.enable` CDP commands
- JS execution via `Page.createIsolatedWorld` → `Runtime.evaluate(contextId)` only
- Page init sends only: `Page.enable`, `Page.getFrameTree`, `Page.setLifecycleEventsEnabled`
- Pipe protocol: JSON + `\0` null byte delimiter on stdin/stdout
- Chrome launched with `--remote-debugging-pipe` (NOT `--remote-debugging-port`)
- All existing stealth patches preserved (args.rs, stealth.rs, shadow_dom.rs)
- Public API unchanged: `Browser::launch`, `Page::goto/evaluate/click/fill/screenshot`
- Crate name: `patchright-rs`, binary name: `patchright`
- Windows + Linux + macOS support for pipe communication

---

### Task 1: Project Cleanup + New Cargo.toml

**Files:**
- Modify: `Cargo.toml`
- Delete: `vendor/chromiumoxide/` (entire directory)
- Delete: `src/cdp/filter.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: Clean project skeleton ready for new CDP implementation

- [ ] **Step 1: Remove chromiumoxide dependency and vendor directory**

Delete `vendor/chromiumoxide/` directory entirely. Remove `chromiumoxide` from Cargo.toml dependencies and the `[patch.crates-io]` section.

- [ ] **Step 2: Update Cargo.toml with new dependencies**

```toml
[package]
name = "patchright-rs"
version = "0.2.0"
edition = "2021"
description = "Undetected browser automation for Rust - pipe-based CDP"
license = "Apache-2.0"

[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"
futures = "0.3"
which = "7"
base64 = "0.22"
clap = { version = "4", features = ["derive"] }
reqwest = { version = "0.12", features = ["stream", "rustls-tls"], default-features = false }
zip = "2"
tokio-util = { version = "0.7", features = ["io"] }

[dev-dependencies]
tokio = { version = "1", features = ["full", "test-util"] }
tracing-subscriber = "0.3"

[[bin]]
name = "patchright"
path = "src/bin/patchright.rs"
```

- [ ] **Step 3: Strip src/lib.rs to minimal module declarations**

```rust
pub mod cdp;
pub mod browser;
pub mod page;
pub mod patches;
pub mod config;
pub mod error;

pub use browser::Browser;
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{PatchrightError, Result};
pub use page::Page;
```

- [ ] **Step 4: Delete old cdp/filter.rs, verify `cargo check` compiles (with stubs)**

Create placeholder modules so it compiles. Each subsequent task fills them in.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: remove chromiumoxide, prepare for pipe-based CDP"
```

---

### Task 2: PipeTransport (cdp/pipe.rs)

**Files:**
- Create: `src/cdp/pipe.rs`
- Create: `src/cdp/mod.rs`
- Test: inline `#[cfg(test)]` in pipe.rs

**Interfaces:**
- Produces: `PipeTransport::new(stdin, stdout)`, `PipeTransport::send(&Value)`, `PipeTransport::recv() -> Value`

- [ ] **Step 1: Write PipeTransport with unit tests**

```rust
// src/cdp/pipe.rs
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use serde_json::Value;
use crate::error::{PatchrightError, Result};

/// Pipe-based CDP transport using Chrome's stdin/stdout.
/// Messages are JSON objects delimited by null bytes (\0).
pub struct PipeTransport {
    writer: ChildStdin,
    reader: BufReader<ChildStdout>,
}

impl PipeTransport {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            writer: stdin,
            reader: BufReader::new(stdout),
        }
    }

    /// Send a CDP message (JSON + null byte)
    pub async fn send(&mut self, msg: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(msg)
            .map_err(|e| PatchrightError::CdpError(format!("serialize: {e}")))?;
        bytes.push(0);
        self.writer.write_all(&bytes).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Receive next CDP message (read until null byte)
    pub async fn recv(&mut self) -> Result<Value> {
        let mut buf = Vec::new();
        let n = self.reader.read_until(0, &mut buf).await?;
        if n == 0 {
            return Err(PatchrightError::CdpError("pipe closed".into()));
        }
        if buf.last() == Some(&0) {
            buf.pop();
        }
        if buf.is_empty() {
            return Err(PatchrightError::CdpError("empty message".into()));
        }
        serde_json::from_slice(&buf)
            .map_err(|e| PatchrightError::CdpError(format!("deserialize: {e}")))
    }
}
```

- [ ] **Step 2: Write cdp/mod.rs**

```rust
pub mod pipe;
pub mod session;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check`

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(cdp): PipeTransport - null-byte delimited JSON over stdin/stdout"
```

---

### Task 3: CdpSession (cdp/session.rs)

**Files:**
- Create: `src/cdp/session.rs`

**Interfaces:**
- Consumes: `PipeTransport`
- Produces: `CdpSession::execute(method, params) -> Result<Value>`, `CdpSession::spawn_reader()`

- [ ] **Step 1: Write CdpSession**

```rust
// src/cdp/session.rs
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot, Mutex};
use tokio::task::JoinHandle;
use serde_json::{json, Value};
use crate::cdp::pipe::PipeTransport;
use crate::error::{PatchrightError, Result};

#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

pub struct CdpSession {
    transport: Arc<Mutex<PipeTransport>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    event_tx: broadcast::Sender<CdpEvent>,
}

impl CdpSession {
    pub fn new(transport: PipeTransport) -> Arc<Self> {
        let (event_tx, _) = broadcast::channel(256);
        Arc::new(Self {
            transport: Arc::new(Mutex::new(transport)),
            next_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
        })
    }

    /// Send a CDP command and wait for its response.
    pub async fn execute(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let msg = json!({ "id": id, "method": method, "params": params });
        self.transport.lock().await.send(&msg).await?;

        let response = rx.await
            .map_err(|_| PatchrightError::CdpError("response channel closed".into()))?;

        if let Some(error) = response.get("error") {
            let msg = error.get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown CDP error");
            return Err(PatchrightError::CdpError(msg.to_string()));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Subscribe to CDP events.
    pub fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.event_tx.subscribe()
    }

    /// Spawn background reader loop that routes responses and events.
    pub fn spawn_reader(self: &Arc<Self>) -> JoinHandle<()> {
        let transport = Arc::clone(&self.transport);
        let pending = Arc::clone(&self.pending);
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            loop {
                let msg = {
                    let mut t = transport.lock().await;
                    match t.recv().await {
                        Ok(m) => m,
                        Err(_) => break, // pipe closed
                    }
                };

                if let Some(id) = msg.get("id").and_then(|i| i.as_u64()) {
                    // Response to a command
                    let mut p = pending.lock().await;
                    if let Some(tx) = p.remove(&id) {
                        let _ = tx.send(msg);
                    }
                } else if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                    // Event
                    let event = CdpEvent {
                        method: method.to_string(),
                        params: msg.get("params").cloned().unwrap_or(Value::Null),
                        session_id: msg.get("sessionId").and_then(|s| s.as_str()).map(String::from),
                    };
                    let _ = event_tx.send(event);
                }
            }
        })
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check`

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(cdp): CdpSession - command routing, event broadcast, background reader"
```

---

### Task 4: Browser Launch with Pipe (browser/)

**Files:**
- Rewrite: `src/browser/mod.rs`
- Rewrite: `src/browser/launch.rs`

**Interfaces:**
- Consumes: `CdpSession`, `PipeTransport`, `LaunchOptions`, `patches::args`
- Produces: `Browser::launch(options) -> Result<Browser>`, `Browser::new_page()`, `Browser::close()`

- [ ] **Step 1: Rewrite browser/launch.rs (keep resolve_executable, update build_stealth_args)**

Keep `resolve_executable()` unchanged. Keep `build_stealth_args()` unchanged (already produces args without `--` prefix). Add `--remote-debugging-pipe` in the launch function.

- [ ] **Step 2: Rewrite browser/mod.rs with pipe-based launch**

```rust
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
        // Add --remote-debugging-pipe (with -- prefix for Chrome)
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
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check`

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(browser): pipe-based launch with --remote-debugging-pipe"
```

---

### Task 5: Page API (page/)

**Files:**
- Rewrite: `src/page/mod.rs`
- Rewrite: `src/page/evaluate.rs`
- Rewrite: `src/page/navigate.rs`
- Rewrite: `src/page/screenshot.rs`

**Interfaces:**
- Consumes: `CdpSession`, `patches::stealth`, `patches::shadow_dom`
- Produces: `Page::create()`, `Page::goto()`, `Page::evaluate()`, `Page::click()`, `Page::fill()`, `Page::screenshot()`

- [ ] **Step 1: Rewrite page/mod.rs with Page::create using Target.createTarget + attachToTarget**

Page creation flow:
1. `Target.createTarget { url: "about:blank" }` → targetId
2. `Target.attachToTarget { targetId, flatten: true }` → sessionId
3. `Page.enable` on the session
4. `Page.setLifecycleEventsEnabled { enabled: true }`
5. Inject stealth scripts via `Page.addScriptToEvaluateOnNewDocument`
6. Do NOT send `Runtime.enable`

- [ ] **Step 2: Rewrite page/evaluate.rs using Page.createIsolatedWorld + Runtime.evaluate**

Same logic as before but using our own CdpSession.execute() instead of chromiumoxide.

- [ ] **Step 3: Rewrite page/navigate.rs using Page.navigate + wait for load event**

- [ ] **Step 4: Rewrite page/screenshot.rs using Page.captureScreenshot + base64 decode**

- [ ] **Step 5: Rewrite src/element/mod.rs using evaluate() for click/fill/wait**

- [ ] **Step 6: Verify compilation and run tests**

Run: `cargo check` then `cargo test`

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(page): rebuild Page API on pipe-based CDP session"
```

---

### Task 6: Integration Test - Browser Launch + Evaluate

**Files:**
- Modify: `tests/integration.rs`

**Interfaces:**
- Consumes: All public API

- [ ] **Step 1: Write integration test that launches browser via pipe and evaluates JS**

```rust
#[tokio::test]
async fn test_pipe_browser_launch_and_evaluate() {
    let Some(browser) = launch_test_browser().await else { return; };
    let page = browser.new_page().await.unwrap();
    page.goto("about:blank").await.unwrap();
    let result = page.evaluate("1 + 2").await.unwrap();
    assert_eq!(result, serde_json::json!(3));
    browser.close().await.unwrap();
}
```

- [ ] **Step 2: Run test**

Run: `cargo test --test integration -- --nocapture`
Expected: PASS (browser launches via pipe, evaluates JS)

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "test: integration test for pipe-based browser launch"
```

---

### Task 7: Stealth Verification - Browserscan

**Files:**
- Modify: `tests/stealth.rs`
- Create: `examples/verify_browserscan.rs`

**Interfaces:**
- Consumes: Full public API

- [ ] **Step 1: Run Browserscan verification in headed mode**

Launch browser headed, navigate to browserscan.net/bot-detection, wait 10s, take screenshot, read page verdict via JS.

- [ ] **Step 2: Verify Browserscan does NOT show "Robot"**

Expected: Page verdict is "Normal" or not "Robot"

- [ ] **Step 3: Run Sannysoft verification**

Expected: navigator.webdriver check passes

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "test: Browserscan + Sannysoft stealth verification"
```

---

### Task 8: CLI - Core Commands (src/bin/patchright.rs)

**Files:**
- Create: `src/bin/patchright.rs`

**Interfaces:**
- Consumes: `Browser`, `LaunchOptions`, `Page`
- Produces: `patchright` binary with subcommands

- [ ] **Step 1: Write CLI with clap derive**

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "patchright", version, about = "Undetected browser automation")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install browser binaries
    Install { browser: Option<String> },
    /// Open a URL in headed mode
    Open { url: String },
    /// Take a screenshot
    Screenshot { url: String, output: String },
    /// Generate PDF from page
    Pdf { url: String, output: String },
    /// Run a JavaScript automation script
    Run { script: std::path::PathBuf },
    /// Record interactions and generate code
    Codegen { url: String },
}
```

- [ ] **Step 2: Implement `screenshot` command**

Launch headed browser, navigate, screenshot, close.

- [ ] **Step 3: Implement `open` command**

Launch headed browser, navigate, keep open until Ctrl+C.

- [ ] **Step 4: Implement `pdf` command**

Use `Page.printToPDF` CDP command.

- [ ] **Step 5: Verify `cargo build` produces working binary**

Run: `cargo run --bin patchright -- screenshot https://example.com test.png`

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(cli): patchright binary with screenshot, open, pdf commands"
```

---

### Task 9: CLI - Browser Install (patchright install)

**Files:**
- Create: `src/cli/install.rs`
- Modify: `src/bin/patchright.rs`

**Interfaces:**
- Produces: `patchright install chrome` downloads and extracts Chrome for Testing

- [ ] **Step 1: Implement browser download from Chrome for Testing CDN**

URL: `https://storage.googleapis.com/chrome-for-testing-public/{version}/{platform}/chrome-{platform}.zip`
Install to: `~/.patchright/browsers/chrome-{version}/`
Record in: `~/.patchright/browsers.json`

- [ ] **Step 2: Implement zip extraction**

- [ ] **Step 3: Update resolve_executable to check ~/.patchright/browsers/ first**

- [ ] **Step 4: Test install command**

Run: `cargo run --bin patchright -- install chrome`

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(cli): patchright install - download Chrome for Testing"
```

---

### Task 10: CLI - Run Script + Codegen

**Files:**
- Create: `src/cli/run.rs`
- Create: `src/cli/codegen.rs`

**Interfaces:**
- Produces: `patchright run script.js`, `patchright codegen <url>`

- [ ] **Step 1: Implement `run` command (execute JS via CDP Runtime.evaluate)**

Read script file, launch browser, execute script content via evaluate, return results.

- [ ] **Step 2: Implement `codegen` command (listen to CDP events, generate Rust code)**

Listen for DOM events (click, input), generate corresponding patchright-rs code.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(cli): run script and codegen commands"
```

---

### Task 11: Final Verification + Cleanup

**Files:**
- Modify: various (cleanup unused code, update docs)

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All pass

- [ ] **Step 2: Run Browserscan headed verification**

Expected: Not "Robot"

- [ ] **Step 3: Run `patchright screenshot` end-to-end**

Expected: PNG file created

- [ ] **Step 4: Remove unused examples and debug files**

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "feat: patchright-rs v2 complete - pipe CDP + CLI + stealth verified"
```
