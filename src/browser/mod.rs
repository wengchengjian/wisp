//! Browser process management. Launches Chrome directly with stealth args.

pub mod launch;
pub mod page;
pub mod cdp;
pub mod patches;
pub mod element;

pub use page::Page;
pub use cdp::CdpSession;

use std::sync::Arc;
use std::path::PathBuf;
use std::process::Stdio;

use serde_json::json;
use tokio::process::Child;

use crate::config::LaunchOptions;
use crate::error::{WispError, Result};


pub struct Browser {
    session: Arc<CdpSession>,
    process: Child,
    #[allow(dead_code)]
    user_data_dir: PathBuf,
    headless: bool,
}

impl Browser {
    /// Launch browser with anti-detection patches.
    pub async fn launch(options: LaunchOptions) -> Result<Self> {
        let executable = launch::resolve_executable(&options)?;
        let user_data_dir = options.user_data_dir.clone()
            .unwrap_or_else(|| std::env::temp_dir().join(format!("wisp-{}-{}", std::process::id(), rand_suffix())));

        // Clean up stale DevToolsActivePort from previous runs
        let port_file = user_data_dir.join("DevToolsActivePort");
        let _ = std::fs::remove_file(&port_file);

        let mut args = launch::build_stealth_args(&options);
        args.push("remote-debugging-port=0".to_string());
        if options.headless {
            args.push("headless=new".to_string());
        }
        args.push(format!("user-data-dir={}", user_data_dir.display()));

        let chrome_args: Vec<String> = args.iter().map(|a| format!("--{a}")).collect();

        let mut cmd = tokio::process::Command::new(&executable);
        cmd.args(&chrome_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // On Windows: control window visibility via creation flags
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            if options.headless {
                cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            } else {
                cmd.creation_flags(0x00000000); // Explicitly clear to prevent auto CREATE_NO_WINDOW
            }
        }

        let child = cmd.spawn()
            .map_err(|e| WispError::LaunchFailed(format!("spawn chrome: {e}")))?;

        // Wait for DevToolsActivePort file (contains random port + ws path)
        let ws_url = Self::wait_for_devtools_url(&user_data_dir).await?;
        tracing::info!("Chrome DevTools: {}", ws_url);

        let session = CdpSession::connect(&ws_url).await?;
        Ok(Self { session, process: child, user_data_dir, headless: options.headless })
    }

    async fn wait_for_devtools_url(user_data_dir: &PathBuf) -> Result<String> {
        let port_file = user_data_dir.join("DevToolsActivePort");
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);

        loop {
            if port_file.exists() {
                // Small delay to ensure file is fully written
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let content = tokio::fs::read_to_string(&port_file).await
                    .map_err(|e| WispError::LaunchFailed(format!("read DevToolsActivePort: {e}")))?;
                let mut lines = content.lines();
                let port = lines.next().ok_or_else(|| WispError::LaunchFailed("empty DevToolsActivePort".into()))?.trim();
                let ws_path = lines.next().unwrap_or("/devtools/browser");
                return Ok(format!("ws://127.0.0.1:{}{}", port, ws_path));
            }
            if tokio::time::Instant::now() > deadline {
                return Err(WispError::LaunchFailed("Chrome did not start within 15s".into()));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Create a new page (tab).
    pub async fn new_page(&self) -> Result<Page> {
        Page::create(Arc::clone(&self.session), self.headless).await
    }

    /// Close the browser.
    pub async fn close(mut self) -> Result<()> {
        // 先尝试优雅关闭（CDP Browser.close）
        if let Err(e) = self.session.execute("Browser.close", json!({})).await {
            tracing::warn!("CDP Browser.close 失败: {}，回退到 kill", e);
        }
        // 无论 CDP 是否成功，确保进程被 kill（close 消费 self，Drop 不再运行）
        let _ = self.process.start_kill();
        // 等待进程退出（最多 3 秒）
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            self.process.wait()
        ).await;
        Ok(())
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.process.start_kill();
        // 清理临时 user_data_dir（仅清理我们创建的，以 wisp- 开头）
        if let Some(dir) = self.user_data_dir.to_str() {
            if dir.contains("wisp-") {
                let dir = self.user_data_dir.clone();
                // 在独立线程清理，避免阻塞 Drop
                std::thread::spawn(move || {
                    let _ = std::fs::remove_dir_all(&dir);
                });
            }
        }
    }
}

/// Generate a short random suffix for unique temp dirs.
fn rand_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos();
    format!("{:x}", nanos)
}
