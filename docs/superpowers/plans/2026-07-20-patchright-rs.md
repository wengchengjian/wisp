# Patchright-RS Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust crate that controls Chromium via CDP with anti-detection patches equivalent to patchright.

**Architecture:** Wraps chromiumoxide for browser process management and CDP WebSocket transport. All page-level operations use raw CDP commands to avoid sending `Runtime.enable`/`Console.enable`. Launch args are patched before browser spawn. JS executes in isolated worlds via `Page.createIsolatedWorld`.

**Tech Stack:** Rust, tokio, chromiumoxide, serde/serde_json, thiserror, tracing

## Global Constraints

- Only Chromium-based browsers supported (Chrome, Edge, Chromium)
- `console.log` intentionally non-functional (Console.enable never sent)
- No network interception, file upload/download, or multi-tab in v1
- All CDP communication must avoid `Runtime.enable` and `Console.enable`
- Async API using tokio runtime
- Crate name: `patchright-rs`, lib name: `patchright_rs`

---

### Task 1: Project Skeleton

**Files:**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/error.rs`
- Create: `src/config.rs`

**Interfaces:**
- Produces: `PatchrightError`, `Result<T>`, `LaunchOptions`, `ProxyConfig`

- [ ] **Step 1: Initialize Cargo project**

```bash
cd f:\project\patchright-rs
cargo init --lib --name patchright-rs
```

- [ ] **Step 2: Write Cargo.toml**

```toml
[package]
name = "patchright-rs"
version = "0.1.0"
edition = "2021"
description = "Undetected browser automation for Rust - CDP-based anti-detection library"
license = "Apache-2.0"

[dependencies]
chromiumoxide = { version = "0.7", features = ["tokio-runtime"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"
futures = "0.3"
which = "7"

[dev-dependencies]
tokio = { version = "1", features = ["full", "test-util"] }
```

- [ ] **Step 3: Write src/error.rs**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PatchrightError {
    #[error("Browser launch failed: {0}")]
    LaunchFailed(String),

    #[error("CDP connection error: {0}")]
    CdpError(String),

    #[error("Navigation failed: {0}")]
    NavigationFailed(String),

    #[error("Element not found: {selector}")]
    ElementNotFound { selector: String },

    #[error("JS evaluation error: {0}")]
    EvalError(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("CDP error: {0}")]
    CdpProtocol(String),
}

pub type Result<T> = std::result::Result<T, PatchrightError>;
```

- [ ] **Step 4: Write src/config.rs**

```rust
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub headless: bool,
    pub channel: Option<String>,
    pub executable_path: Option<PathBuf>,
    pub user_data_dir: Option<PathBuf>,
    pub no_viewport: bool,
    pub args: Vec<String>,
    pub proxy: Option<ProxyConfig>,
    pub timeout: Duration,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            headless: false,
            channel: None,
            executable_path: None,
            user_data_dir: None,
            no_viewport: false,
            args: Vec::new(),
            proxy: None,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub server: String,
    pub username: Option<String>,
    pub password: Option<String>,
}
```

- [ ] **Step 5: Write src/lib.rs**

```rust
pub mod config;
pub mod error;

pub use config::{LaunchOptions, ProxyConfig};
pub use error::{PatchrightError, Result};
```

- [ ] **Step 6: Verify compilation**

Run: `cargo check`
Expected: compiles with no errors (warnings about unused deps OK)

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: project skeleton with error types and config"
```

---

### Task 2: Launch Args Patch

**Files:**
- Create: `src/patches/mod.rs`
- Create: `src/patches/args.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: `patches::args::patch_launch_args(args: &mut Vec<String>)`

- [ ] **Step 1: Write test for args patching**

Add to `src/patches/args.rs`:

```rust
/// Flags that leak automation identity and must be removed.
const REMOVE_ARGS: &[&str] = &[
    "--enable-automation",
    "--disable-popup-blocking",
    "--disable-component-update",
    "--disable-default-apps",
    "--disable-extensions",
];

/// Flags that must be added for stealth.
const ADD_ARGS: &[&str] = &[
    "--disable-blink-features=AutomationControlled",
];

/// Patch browser launch arguments to remove detection vectors.
/// Removes automation-revealing flags and adds stealth flags.
pub fn patch_launch_args(args: &mut Vec<String>) {
    args.retain(|a| !REMOVE_ARGS.contains(&a.as_str()));
    for arg in ADD_ARGS {
        if !args.iter().any(|a| a == arg) {
            args.push(arg.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_removes_automation_flags() {
        let mut args = vec![
            "--enable-automation".to_string(),
            "--disable-popup-blocking".to_string(),
            "--disable-component-update".to_string(),
            "--disable-default-apps".to_string(),
            "--disable-extensions".to_string(),
            "--no-first-run".to_string(),
        ];
        patch_launch_args(&mut args);
        assert!(!args.contains(&"--enable-automation".to_string()));
        assert!(!args.contains(&"--disable-popup-blocking".to_string()));
        assert!(!args.contains(&"--disable-component-update".to_string()));
        assert!(!args.contains(&"--disable-default-apps".to_string()));
        assert!(!args.contains(&"--disable-extensions".to_string()));
        assert!(args.contains(&"--no-first-run".to_string()));
    }

    #[test]
    fn test_adds_stealth_flags() {
        let mut args = vec!["--no-first-run".to_string()];
        patch_launch_args(&mut args);
        assert!(args.contains(&"--disable-blink-features=AutomationControlled".to_string()));
    }

    #[test]
    fn test_no_duplicate_stealth_flags() {
        let mut args = vec![
            "--disable-blink-features=AutomationControlled".to_string(),
        ];
        patch_launch_args(&mut args);
        let count = args.iter()
            .filter(|a| *a == "--disable-blink-features=AutomationControlled")
            .count();
        assert_eq!(count, 1);
    }
}
```

- [ ] **Step 2: Write src/patches/mod.rs**

```rust
pub mod args;
```

- [ ] **Step 3: Update src/lib.rs**

```rust
pub mod config;
pub mod error;
pub mod patches;

pub use config::{LaunchOptions, ProxyConfig};
pub use error::{PatchrightError, Result};
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: 3 tests pass

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: launch args patch - remove automation flags, add stealth flags"
```

---

### Task 3: Browser Launch Module

**Files:**
- Create: `src/browser/mod.rs`
- Create: `src/browser/launch.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `LaunchOptions`, `patches::args::patch_launch_args`
- Produces: `Browser` struct with `launch()`, `new_page()`, `close()`

- [ ] **Step 1: Write src/browser/launch.rs**

```rust
use std::path::PathBuf;

use crate::config::LaunchOptions;
use crate::error::{PatchrightError, Result};
use crate::patches;

/// Resolve the browser executable path from options.
/// Priority: executable_path > channel lookup > system chromium
pub fn resolve_executable(options: &LaunchOptions) -> Result<PathBuf> {
    // 1. Explicit path
    if let Some(ref path) = options.executable_path {
        if path.exists() {
            return Ok(path.clone());
        }
        return Err(PatchrightError::LaunchFailed(format!(
            "Executable not found: {}",
            path.display()
        )));
    }

    // 2. Channel-based lookup
    let names: Vec<&str> = match options.channel.as_deref() {
        Some("chrome") => vec!["chrome", "google-chrome", "google-chrome-stable"],
        Some("msedge") => vec!["msedge", "microsoft-edge"],
        Some("chromium") => vec!["chromium", "chromium-browser"],
        None => vec!["chrome", "google-chrome", "chromium", "chromium-browser", "msedge"],
        Some(other) => vec![other],
    };

    // Try well-known Windows paths first
    if cfg!(target_os = "windows") {
        let windows_paths = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        ];
        for p in &windows_paths {
            let path = PathBuf::from(p);
            if path.exists() {
                return Ok(path);
            }
        }
    }

    // Try `which` lookup
    for name in &names {
        if let Ok(path) = which::which(name) {
            return Ok(path);
        }
    }

    Err(PatchrightError::LaunchFailed(
        "No Chromium-based browser found. Install Chrome/Chromium/Edge or set executable_path.".into(),
    ))
}

/// Build default Chrome launch arguments from options.
pub fn build_default_args(options: &LaunchOptions) -> Vec<String> {
    let mut args = Vec::new();

    if options.headless {
        args.push("--headless=new".to_string());
    }

    if options.no_viewport {
        args.push("--no-default-browser-check".to_string());
    } else {
        args.push("--window-size=1280,720".to_string());
    }

    if let Some(ref user_data_dir) = options.user_data_dir {
        args.push(format!("--user-data-dir={}", user_data_dir.display()));
    }

    if let Some(ref proxy) = options.proxy {
        args.push(format!("--proxy-server={}", proxy.server));
    }

    // Standard stealth-friendly defaults
    args.push("--no-first-run".to_string());
    args.push("--no-default-browser-check".to_string());
    args.push("--disable-background-networking".to_string());
    args.push("--disable-sync".to_string());
    args.push("--disable-translate".to_string());
    args.push("--metrics-recording-only".to_string());
    args.push("--safebrowsing-disable-auto-update".to_string());

    // User-provided extra args
    args.extend(options.args.clone());

    // Apply patchright patches
    patches::args::patch_launch_args(&mut args);

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_default_args_headless() {
        let opts = LaunchOptions {
            headless: true,
            ..Default::default()
        };
        let args = build_default_args(&opts);
        assert!(args.contains(&"--headless=new".to_string()));
    }

    #[test]
    fn test_build_default_args_no_automation_flag() {
        let opts = LaunchOptions::default();
        let args = build_default_args(&opts);
        assert!(!args.contains(&"--enable-automation".to_string()));
        assert!(args.contains(&"--disable-blink-features=AutomationControlled".to_string()));
    }

    #[test]
    fn test_build_default_args_user_data_dir() {
        let opts = LaunchOptions {
            user_data_dir: Some(PathBuf::from("./test-profile")),
            ..Default::default()
        };
        let args = build_default_args(&opts);
        assert!(args.iter().any(|a| a.starts_with("--user-data-dir=")));
    }

    #[test]
    fn test_build_default_args_proxy() {
        let opts = LaunchOptions {
            proxy: Some(crate::config::ProxyConfig {
                server: "http://127.0.0.1:8080".into(),
                username: None,
                password: None,
            }),
            ..Default::default()
        };
        let args = build_default_args(&opts);
        assert!(args.contains(&"--proxy-server=http://127.0.0.1:8080".to_string()));
    }
}
```

- [ ] **Step 2: Write src/browser/mod.rs**

```rust
pub mod launch;

use std::process::Child;

use chromiumoxide::browser::Browser as CdpBrowser;
use chromiumoxide::browser::BrowserConfig;
use tokio::task::JoinHandle;

use crate::config::LaunchOptions;
use crate::error::{PatchrightError, Result};
use crate::page::Page;

/// A patched Chromium browser instance.
/// All CDP communication avoids Runtime.enable and Console.enable.
pub struct Browser {
    inner: CdpBrowser,
    handle: JoinHandle<()>,
    #[allow(dead_code)]
    options: LaunchOptions,
}

impl Browser {
    /// Launch a new browser instance with anti-detection patches applied.
    pub async fn launch(options: LaunchOptions) -> Result<Self> {
        let executable = launch::resolve_executable(&options)?;
        let args = launch::build_default_args(&options);

        let config = BrowserConfig::builder()
            .chrome_executable(executable)
            .custom_args(args)
            .no_sandbox()
            .build()
            .map_err(|e| PatchrightError::LaunchFailed(e.to_string()))?;

        let (inner, handle) = CdpBrowser::launch(config)
            .await
            .map_err(|e| PatchrightError::LaunchFailed(e.to_string()))?;

        Ok(Self { inner, handle, options })
    }

    /// Create a new page (tab) in the browser.
    pub async fn new_page(&self) -> Result<Page> {
        let cdp_page = self
            .inner
            .new_page("about:blank")
            .await
            .map_err(|e| PatchrightError::CdpError(e.to_string()))?;

        Page::new(cdp_page).await
    }

    /// Close the browser and all its pages.
    pub async fn close(self) -> Result<()> {
        self.inner
            .close()
            .await
            .map_err(|e| PatchrightError::CdpError(e.to_string()))?;
        let _ = self.handle.await;
        Ok(())
    }
}
```

- [ ] **Step 3: Create placeholder src/page/mod.rs (minimal for compilation)**

```rust
use chromiumoxide::Page as CdpPage;

use crate::error::Result;

/// A browser page (tab) with anti-detection patches.
pub struct Page {
    pub(crate) inner: CdpPage,
}

impl Page {
    pub(crate) async fn new(inner: CdpPage) -> Result<Self> {
        Ok(Self { inner })
    }
}
```

- [ ] **Step 4: Update src/lib.rs**

```rust
pub mod browser;
pub mod config;
pub mod error;
pub mod page;
pub mod patches;

pub use browser::Browser;
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{PatchrightError, Result};
pub use page::Page;
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check`
Expected: compiles (chromiumoxide API may need version adjustments)

- [ ] **Step 6: Run unit tests**

Run: `cargo test`
Expected: all args/launch tests pass

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: browser launch module with executable resolution and arg building"
```

---

### Task 4: CDP Filter Layer

**Files:**
- Create: `src/cdp/mod.rs`
- Create: `src/cdp/filter.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: `cdp::filter::should_block(method: &str) -> bool`

- [ ] **Step 1: Write src/cdp/filter.rs with tests**

```rust
/// CDP methods that must NEVER be sent to avoid detection.
/// These are the core patchright patches at the protocol level.
const BLOCKED_METHODS: &[&str] = &[
    "Runtime.enable",
    "Console.enable",
];

/// Returns true if the given CDP method should be blocked (never sent).
pub fn should_block(method: &str) -> bool {
    BLOCKED_METHODS.contains(&method)
}

/// Returns true if the given CDP method is safe to send.
pub fn is_allowed(method: &str) -> bool {
    !should_block(method)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocks_runtime_enable() {
        assert!(should_block("Runtime.enable"));
    }

    #[test]
    fn test_blocks_console_enable() {
        assert!(should_block("Console.enable"));
    }

    #[test]
    fn test_allows_runtime_evaluate() {
        assert!(!should_block("Runtime.evaluate"));
    }

    #[test]
    fn test_allows_page_navigate() {
        assert!(!should_block("Page.navigate"));
    }

    #[test]
    fn test_allows_page_create_isolated_world() {
        assert!(!should_block("Page.createIsolatedWorld"));
    }

    #[test]
    fn test_is_allowed_inverse() {
        assert!(!is_allowed("Runtime.enable"));
        assert!(is_allowed("Runtime.evaluate"));
    }
}
```

- [ ] **Step 2: Write src/cdp/mod.rs**

```rust
pub mod filter;
```

- [ ] **Step 3: Update src/lib.rs to include cdp module**

Add `pub mod cdp;` to lib.rs.

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: all filter tests pass

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: CDP command filter layer - blocks Runtime.enable and Console.enable"
```

---

### Task 5: Page Navigation

**Files:**
- Modify: `src/page/mod.rs`
- Create: `src/page/navigate.rs`

**Interfaces:**
- Consumes: `Page.inner` (CdpPage)
- Produces: `Page::goto()`, `Page::go_back()`, `Page::reload()`

- [ ] **Step 1: Write src/page/navigate.rs**

```rust
use chromiumoxide::cdp::browser_protocol::page::NavigateParams;
use chromiumoxide::Page as CdpPage;

use crate::error::{PatchrightError, Result};

/// Navigate the page to a URL.
pub async fn goto(page: &CdpPage, url: &str) -> Result<()> {
    page.goto(url)
        .await
        .map_err(|e| PatchrightError::NavigationFailed(e.to_string()))?;
    // Wait for load event
    page.wait_for_navigation()
        .await
        .map_err(|e| PatchrightError::NavigationFailed(e.to_string()))?;
    Ok(())
}

/// Navigate back in history.
pub async fn go_back(page: &CdpPage) -> Result<()> {
    page.execute(chromiumoxide::cdp::browser_protocol::page::NavigateParams {
        url: "javascript:history.back()".to_string(),
        ..Default::default()
    })
    .await
    .map_err(|e| PatchrightError::NavigationFailed(e.to_string()))?;
    Ok(())
}

/// Reload the current page.
pub async fn reload(page: &CdpPage) -> Result<()> {
    page.reload()
        .await
        .map_err(|e| PatchrightError::NavigationFailed(e.to_string()))?;
    Ok(())
}
```

- [ ] **Step 2: Update src/page/mod.rs**

```rust
pub mod navigate;

use chromiumoxide::Page as CdpPage;

use crate::error::Result;

/// A browser page (tab) with anti-detection patches.
/// JS execution uses isolated ExecutionContexts (no Runtime.enable).
pub struct Page {
    pub(crate) inner: CdpPage,
}

impl Page {
    pub(crate) async fn new(inner: CdpPage) -> Result<Self> {
        Ok(Self { inner })
    }

    /// Navigate to a URL and wait for load.
    pub async fn goto(&self, url: &str) -> Result<()> {
        navigate::goto(&self.inner, url).await
    }

    /// Navigate back in history.
    pub async fn go_back(&self) -> Result<()> {
        navigate::go_back(&self.inner).await
    }

    /// Reload the current page.
    pub async fn reload(&self) -> Result<()> {
        navigate::reload(&self.inner).await
    }
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check`
Expected: compiles (adjust chromiumoxide API calls if needed)

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: page navigation - goto, go_back, reload"
```

---

### Task 6: JS Evaluation with Isolated ExecutionContext

**Files:**
- Create: `src/page/evaluate.rs`
- Modify: `src/page/mod.rs`

**Interfaces:**
- Consumes: `Page.inner`, `cdp::filter`
- Produces: `Page::evaluate(expr) -> Result<serde_json::Value>`

This is the CORE patchright patch: execute JS without Runtime.enable.

- [ ] **Step 1: Write src/page/evaluate.rs**

```rust
use chromiumoxide::cdp::browser_protocol::page::CreateIsolatedWorldParams;
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use chromiumoxide::Page as CdpPage;
use serde_json::Value;

use crate::error::{PatchrightError, Result};

/// Evaluate JavaScript in an isolated ExecutionContext.
///
/// This is the core patchright patch: instead of calling Runtime.enable
/// (which is detectable), we create an isolated world via
/// Page.createIsolatedWorld and execute JS there directly.
///
/// Corresponds to patchright's crPagePatch.ts Runtime.enable removal.
pub async fn evaluate(page: &CdpPage, expression: &str) -> Result<Value> {
    // Get the main frame ID
    let frame_tree = page
        .execute(chromiumoxide::cdp::browser_protocol::page::GetFrameTreeParams::default())
        .await
        .map_err(|e| PatchrightError::CdpError(format!("GetFrameTree: {e}")))?;

    let frame_id = frame_tree.frame_tree.frame.id.clone();

    // Create isolated world (does NOT require Runtime.enable)
    let world = page
        .execute(CreateIsolatedWorldParams {
            frame_id,
            world_name: Some("patchright".to_string()),
            grant_universal_access: Some(true),
        })
        .await
        .map_err(|e| PatchrightError::CdpError(format!("CreateIsolatedWorld: {e}")))?;

    let context_id = world.execution_context_id;

    // Evaluate in the isolated context
    let result = page
        .execute(EvaluateParams {
            expression: expression.to_string(),
            context_id: Some(context_id),
            return_by_value: Some(true),
            await_promise: Some(true),
            ..Default::default()
        })
        .await
        .map_err(|e| PatchrightError::EvalError(e.to_string()))?;

    // Check for exceptions
    if let Some(exception) = &result.exception_details {
        let msg = exception
            .exception
            .as_ref()
            .and_then(|e| e.description.clone())
            .unwrap_or_else(|| exception.text.clone());
        return Err(PatchrightError::EvalError(msg));
    }

    Ok(result.result.value.unwrap_or(Value::Null))
}

/// Evaluate JS and return a string representation.
pub async fn evaluate_as_string(page: &CdpPage, expression: &str) -> Result<String> {
    let value = evaluate(page, expression).await?;
    Ok(match value {
        Value::String(s) => s,
        Value::Null => "null".to_string(),
        other => other.to_string(),
    })
}
```

- [ ] **Step 2: Add evaluate methods to Page in src/page/mod.rs**

Add to the `impl Page` block:

```rust
    /// Evaluate JavaScript in an isolated ExecutionContext.
    /// Does NOT send Runtime.enable (core anti-detection patch).
    pub async fn evaluate(&self, expression: &str) -> Result<serde_json::Value> {
        evaluate::evaluate(&self.inner, expression).await
    }

    /// Evaluate JavaScript and return result as String.
    pub async fn evaluate_as_string(&self, expression: &str) -> Result<String> {
        evaluate::evaluate_as_string(&self.inner, expression).await
    }
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check`
Expected: compiles (CDP type names may need adjustment based on chromiumoxide version)

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: JS evaluation via isolated ExecutionContext (no Runtime.enable)"
```

---

### Task 7: Element Finding and Interaction

**Files:**
- Create: `src/element/mod.rs`
- Create: `src/element/selector.rs`
- Modify: `src/page/mod.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `Page::evaluate`, `Page.inner`
- Produces: `Page::click()`, `Page::fill()`, `Page::wait_for_selector()`, `Page::query_selector()`

- [ ] **Step 1: Write src/element/mod.rs**

```rust
use chromiumoxide::Page as CdpPage;
use serde_json::Value;

use crate::error::{PatchrightError, Result};
use crate::page::evaluate;

/// Click an element matching the CSS selector.
pub async fn click(page: &CdpPage, selector: &str) -> Result<()> {
    // Use JS to find and click the element
    let js = format!(
        r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found: {}');
            el.click();
            return true;
        }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );

    evaluate::evaluate(page, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Element not found") => {
            PatchrightError::ElementNotFound {
                selector: selector.to_string(),
            }
        }
        other => other,
    })?;

    Ok(())
}

/// Type text into an input element matching the CSS selector.
pub async fn fill(page: &CdpPage, selector: &str, value: &str) -> Result<()> {
    let js = format!(
        r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found: {}');
            el.focus();
            el.value = {};
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
            el.dispatchEvent(new Event('change', {{ bubbles: true }}));
            return true;
        }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'"),
        serde_json::to_string(value).unwrap()
    );

    evaluate::evaluate(page, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Element not found") => {
            PatchrightError::ElementNotFound {
                selector: selector.to_string(),
            }
        }
        other => other,
    })?;

    Ok(())
}

/// Wait for an element matching the selector to appear in the DOM.
pub async fn wait_for_selector(
    page: &CdpPage,
    selector: &str,
    timeout_ms: u64,
) -> Result<()> {
    let js = format!(
        r#"(async () => {{
            const deadline = Date.now() + {};
            while (Date.now() < deadline) {{
                if (document.querySelector({})) return true;
                await new Promise(r => setTimeout(r, 100));
            }}
            throw new Error('Timeout waiting for: {}');
        }})()"#,
        timeout_ms,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );

    evaluate::evaluate(page, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Timeout") => {
            PatchrightError::Timeout(format!("wait_for_selector: {selector}"))
        }
        other => other,
    })?;

    Ok(())
}

/// Get the text content of an element.
pub async fn text_content(page: &CdpPage, selector: &str) -> Result<String> {
    let js = format!(
        r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found: {}');
            return el.textContent || '';
        }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );

    let value = evaluate::evaluate(page, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Element not found") => {
            PatchrightError::ElementNotFound {
                selector: selector.to_string(),
            }
        }
        other => other,
    })?;

    Ok(value.as_str().unwrap_or("").to_string())
}
```

- [ ] **Step 2: Create src/element/selector.rs (placeholder for future XPath support)**

```rust
/// Selector types supported by patchright-rs.
#[derive(Debug, Clone)]
pub enum Selector {
    Css(String),
    // XPath support can be added later
}

impl From<&str> for Selector {
    fn from(s: &str) -> Self {
        Selector::Css(s.to_string())
    }
}
```

- [ ] **Step 3: Add element methods to Page in src/page/mod.rs**

Add to `impl Page`:

```rust
    /// Click an element matching the CSS selector.
    pub async fn click(&self, selector: &str) -> Result<()> {
        crate::element::click(&self.inner, selector).await
    }

    /// Type text into an input element.
    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> {
        crate::element::fill(&self.inner, selector, value).await
    }

    /// Wait for an element to appear in the DOM.
    pub async fn wait_for_selector(&self, selector: &str, timeout: Option<std::time::Duration>) -> Result<()> {
        let ms = timeout.unwrap_or(std::time::Duration::from_secs(30)).as_millis() as u64;
        crate::element::wait_for_selector(&self.inner, selector, ms).await
    }

    /// Get text content of an element.
    pub async fn text_content(&self, selector: &str) -> Result<String> {
        crate::element::text_content(&self.inner, selector).await
    }
```

- [ ] **Step 4: Update src/lib.rs**

Add `pub mod element;`

- [ ] **Step 5: Verify compilation**

Run: `cargo check`
Expected: compiles

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: element operations - click, fill, wait_for_selector, text_content"
```

---

### Task 8: Screenshot

**Files:**
- Create: `src/page/screenshot.rs`
- Modify: `src/page/mod.rs`

**Interfaces:**
- Consumes: `Page.inner`
- Produces: `Page::screenshot(path)`

- [ ] **Step 1: Write src/page/screenshot.rs**

```rust
use std::path::Path;

use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams;
use chromiumoxide::Page as CdpPage;

use crate::error::{PatchrightError, Result};

/// Capture a screenshot and save to file.
pub async fn screenshot(page: &CdpPage, path: &str) -> Result<()> {
    let params = CaptureScreenshotParams {
        format: Some("png".to_string()),
        ..Default::default()
    };

    let result = page
        .execute(params)
        .await
        .map_err(|e| PatchrightError::CdpError(format!("Screenshot: {e}")))?;

    // CDP returns base64-encoded image data
    let bytes = base64_decode(&result.data)?;
    std::fs::write(Path::new(path), &bytes)?;

    Ok(())
}

/// Capture a screenshot and return raw PNG bytes.
pub async fn screenshot_bytes(page: &CdpPage) -> Result<Vec<u8>> {
    let params = CaptureScreenshotParams {
        format: Some("png".to_string()),
        ..Default::default()
    };

    let result = page
        .execute(params)
        .await
        .map_err(|e| PatchrightError::CdpError(format!("Screenshot: {e}")))?;

    base64_decode(&result.data)
}

fn base64_decode(input: &str) -> Result<Vec<u8>> {
    // Simple base64 decode without extra dependency
    // chromiumoxide may already handle this; adjust as needed
    use std::io::Read;
    let mut decoder = base64_read::Decoder::new(input.as_bytes());
    let mut buf = Vec::new();
    decoder
        .read_to_end(&mut buf)
        .map_err(|e| PatchrightError::CdpError(format!("Base64 decode: {e}")))?;
    Ok(buf)
}
```

Note: If chromiumoxide's `CaptureScreenshot` already returns decoded bytes, simplify accordingly. Alternatively, add `base64 = "0.22"` to dependencies and use `base64::engine::general_purpose::STANDARD.decode()`.

- [ ] **Step 2: Add screenshot methods to Page**

Add to `impl Page`:

```rust
    /// Capture a full-page screenshot and save to file.
    pub async fn screenshot(&self, path: &str) -> Result<()> {
        screenshot::screenshot(&self.inner, path).await
    }

    /// Capture a screenshot and return raw PNG bytes.
    pub async fn screenshot_bytes(&self) -> Result<Vec<u8>> {
        screenshot::screenshot_bytes(&self.inner).await
    }
```

- [ ] **Step 3: Add base64 dependency to Cargo.toml**

```toml
base64 = "0.22"
```

Update `screenshot.rs` to use:
```rust
use base64::Engine;

fn base64_decode(input: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| PatchrightError::CdpError(format!("Base64 decode: {e}")))
}
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: page screenshot capture (PNG)"
```

---

### Task 9: Shadow DOM Patch

**Files:**
- Create: `src/patches/shadow_dom.rs`
- Modify: `src/patches/mod.rs`
- Modify: `src/page/mod.rs`

**Interfaces:**
- Consumes: `Page.inner`
- Produces: Shadow DOM patch auto-injected on page creation

- [ ] **Step 1: Write src/patches/shadow_dom.rs**

```rust
use chromiumoxide::cdp::browser_protocol::page::AddScriptToEvaluateOnNewDocumentParams;
use chromiumoxide::Page as CdpPage;

use crate::error::{PatchrightError, Result};

/// JavaScript that forces all shadow roots to be created as 'open'.
/// This allows standard querySelector to penetrate closed shadow DOMs.
/// Corresponds to patchright's closed shadow root support.
const SHADOW_DOM_PATCH_SCRIPT: &str = r#"
(() => {
    const originalAttachShadow = Element.prototype.attachShadow;
    Element.prototype.attachShadow = function(init) {
        if (init && init.mode === 'closed') {
            init = { ...init, mode: 'open' };
        }
        return originalAttachShadow.call(this, init);
    };
})();
"#;

/// Inject the shadow DOM patch so it runs before any page scripts.
pub async fn inject(page: &CdpPage) -> Result<()> {
    page.execute(AddScriptToEvaluateOnNewDocumentParams {
        source: SHADOW_DOM_PATCH_SCRIPT.to_string(),
        ..Default::default()
    })
    .await
    .map_err(|e| PatchrightError::CdpError(format!("Shadow DOM patch injection: {e}")))?;

    Ok(())
}
```

- [ ] **Step 2: Update src/patches/mod.rs**

```rust
pub mod args;
pub mod shadow_dom;
```

- [ ] **Step 3: Inject shadow DOM patch in Page::new()**

Update `src/page/mod.rs` `Page::new()`:

```rust
    pub(crate) async fn new(inner: CdpPage) -> Result<Self> {
        // Inject shadow DOM patch before any page content loads
        crate::patches::shadow_dom::inject(&inner).await?;
        Ok(Self { inner })
    }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: shadow DOM patch - force closed shadow roots to open"
```

---

### Task 10: Integration Test and Example

**Files:**
- Create: `examples/basic.rs`
- Create: `tests/integration.rs`

**Interfaces:**
- Consumes: All public API

- [ ] **Step 1: Write examples/basic.rs**

```rust
use patchright_rs::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let browser = Browser::launch(LaunchOptions {
        headless: false,
        channel: Some("chrome".into()),
        no_viewport: true,
        ..Default::default()
    })
    .await?;

    let page = browser.new_page().await?;
    page.goto("https://example.com").await?;

    // Verify navigator.webdriver is null (not true)
    let webdriver = page.evaluate("navigator.webdriver").await?;
    println!("navigator.webdriver = {webdriver}");
    assert!(webdriver.is_null(), "webdriver should be null!");

    // Get page title
    let title = page.evaluate_as_string("document.title").await?;
    println!("Page title: {title}");

    // Screenshot
    page.screenshot("example.png").await?;
    println!("Screenshot saved to example.png");

    browser.close().await?;
    println!("Done! Browser closed.");

    Ok(())
}
```

- [ ] **Step 2: Write tests/integration.rs**

```rust
use patchright_rs::{Browser, LaunchOptions};

/// Helper: launch browser for tests. Skips if no Chrome found.
async fn launch_test_browser() -> Option<Browser> {
    Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .ok()
}

#[tokio::test]
async fn test_navigator_webdriver_is_null() {
    let Some(browser) = launch_test_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let page = browser.new_page().await.unwrap();
    page.goto("about:blank").await.unwrap();

    let webdriver = page.evaluate("navigator.webdriver").await.unwrap();
    assert!(
        webdriver.is_null() || webdriver == serde_json::Value::Bool(false),
        "navigator.webdriver should be null or false, got: {webdriver}"
    );

    browser.close().await.unwrap();
}

#[tokio::test]
async fn test_evaluate_returns_value() {
    let Some(browser) = launch_test_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let page = browser.new_page().await.unwrap();
    page.goto("about:blank").await.unwrap();

    let result = page.evaluate("1 + 2").await.unwrap();
    assert_eq!(result, serde_json::json!(3));

    let result = page.evaluate("'hello' + ' ' + 'world'").await.unwrap();
    assert_eq!(result, serde_json::json!("hello world"));

    browser.close().await.unwrap();
}

#[tokio::test]
async fn test_navigation_and_title() {
    let Some(browser) = launch_test_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let page = browser.new_page().await.unwrap();
    page.goto("data:text/html,<title>Test Page</title><h1>Hello</h1>")
        .await
        .unwrap();

    let title = page.evaluate_as_string("document.title").await.unwrap();
    assert_eq!(title, "Test Page");

    browser.close().await.unwrap();
}

#[tokio::test]
async fn test_element_click_and_fill() {
    let Some(browser) = launch_test_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let page = browser.new_page().await.unwrap();
    page.goto("data:text/html,<input id='inp'><button id='btn' onclick='document.getElementById(\"inp\").value=\"clicked\"'>Go</button>")
        .await
        .unwrap();

    page.click("#btn").await.unwrap();
    let value = page.evaluate_as_string("document.getElementById('inp').value").await.unwrap();
    assert_eq!(value, "clicked");

    page.fill("#inp", "typed text").await.unwrap();
    let value = page.evaluate_as_string("document.getElementById('inp').value").await.unwrap();
    assert_eq!(value, "typed text");

    browser.close().await.unwrap();
}

#[tokio::test]
async fn test_screenshot_creates_file() {
    let Some(browser) = launch_test_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let page = browser.new_page().await.unwrap();
    page.goto("data:text/html,<h1>Screenshot Test</h1>")
        .await
        .unwrap();

    let path = std::env::temp_dir().join("patchright_test_screenshot.png");
    let path_str = path.to_str().unwrap();
    page.screenshot(path_str).await.unwrap();

    assert!(path.exists(), "Screenshot file should exist");
    let metadata = std::fs::metadata(&path).unwrap();
    assert!(metadata.len() > 0, "Screenshot should not be empty");

    // Cleanup
    let _ = std::fs::remove_file(&path);
    browser.close().await.unwrap();
}
```

- [ ] **Step 3: Add tracing-subscriber to dev-dependencies**

```toml
[dev-dependencies]
tracing-subscriber = "0.3"
```

- [ ] **Step 4: Run integration tests**

Run: `cargo test --test integration -- --nocapture`
Expected: All tests pass (or SKIP if no Chrome installed)

- [ ] **Step 5: Run the example**

Run: `cargo run --example basic`
Expected: Browser opens, prints webdriver=null, saves screenshot

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: integration tests and basic example"
```

---

### Task 11: Final Polish - Public API Cleanup

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/page/mod.rs`

**Interfaces:**
- Produces: Clean public API surface

- [ ] **Step 1: Finalize src/lib.rs with doc comments**

```rust
//! # patchright-rs
//!
//! Undetected browser automation for Rust.
//! A native Rust implementation of patchright's anti-detection patches,
//! controlling Chromium directly via CDP (Chrome DevTools Protocol).
//!
//! ## Key Patches
//! - No `Runtime.enable` (uses isolated ExecutionContexts)
//! - No `Console.enable` (console disabled by design)
//! - Stealth launch args (no `--enable-automation`)
//! - Closed Shadow Root penetration
//!
//! ## Example
//! ```no_run
//! use patchright_rs::{Browser, LaunchOptions};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let browser = Browser::launch(LaunchOptions::default()).await?;
//!     let page = browser.new_page().await?;
//!     page.goto("https://example.com").await?;
//!     let webdriver = page.evaluate("navigator.webdriver").await?;
//!     assert!(webdriver.is_null());
//!     browser.close().await?;
//!     Ok(())
//! }
//! ```

pub mod browser;
pub mod cdp;
pub mod config;
pub mod element;
pub mod error;
pub mod page;
pub mod patches;

pub use browser::Browser;
pub use config::{LaunchOptions, ProxyConfig};
pub use error::{PatchrightError, Result};
pub use page::Page;
```

- [ ] **Step 2: Verify full build**

Run: `cargo build --release`
Expected: compiles cleanly

- [ ] **Step 3: Run all tests**

Run: `cargo test`
Expected: all pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: finalize public API with documentation"
```
