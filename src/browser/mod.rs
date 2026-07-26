//! Browser process management. Launches Chrome directly with stealth args.

/// CDP WebSocket 会话管理。
pub mod cdp;
/// DOM 元素封装。
pub mod element;
/// Chrome for Testing 自动下载安装。
pub mod installer;
/// 浏览器可执行文件查找与启动参数构建。
pub mod launch;
/// 页面操作（导航、JS 执行、截图等）。
pub mod page;
/// 反检测 JS 补丁注入。
pub mod patches;
/// 浏览器实例池（复用 + 并发控制）。
pub mod pool;

pub use cdp::CdpSession;
pub use page::Page;
pub use pool::BrowserPool;

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use serde_json::json;
use tokio::process::Child;

use crate::config::LaunchOptions;
use crate::error::{BrowserError, Result, WispError};

/// 浏览器实例：封装 Chrome 进程 + CDP 会话。
///
/// 通过 [`Browser::launch`] 启动，使用 [`Browser::new_page`] 创建页面，
/// 最后调用 [`Browser::close`] 关闭。
pub struct Browser {
    session: Arc<CdpSession>,
    process: Child,
    /// 临时目录路径（用于访问）。
    #[allow(dead_code)]
    user_data_path: PathBuf,
    /// 自动清理守卫：Drop 时自动删除临时目录。
    /// 用户自定义 user_data_dir 时为 None（不清理）。
    _temp_dir: Option<tempfile::TempDir>,
    headless: bool,
}

impl Browser {
    /// Launch browser with anti-detection patches.
    ///
    /// # Errors
    ///
    /// - `BrowserError::LaunchFailed` — 找不到 Chrome/Chromium 可执行文件、进程启动失败、
    ///   DevToolsActivePort 超时未出现。
    /// - `BrowserError::CdpConnection` — WebSocket 连接 CDP 失败。
    pub async fn launch(options: LaunchOptions) -> Result<Self> {
        let executable = launch::resolve_executable(&options).await?;

        // 用户自定义目录：不清理；否则创建 TempDir 自动清理。
        let (user_data_path, temp_dir) = if let Some(ref dir) = options.user_data_dir {
            (dir.clone(), None)
        } else {
            let td = tempfile::Builder::new()
                .prefix(&format!("wisp-{}-", std::process::id()))
                .tempdir()
                .map_err(|e| {
                    WispError::Browser(BrowserError::LaunchFailed(format!("create temp dir: {e}")))
                })?;
            let path = td.path().to_path_buf();
            (path, Some(td))
        };

        // Clean up stale DevToolsActivePort from previous runs
        let port_file = user_data_path.join("DevToolsActivePort");
        let _ = std::fs::remove_file(&port_file);

        let mut args = launch::build_stealth_args(&options);
        args.push("remote-debugging-port=0".to_string());
        if options.headless {
            args.push("headless=new".to_string());
        }
        args.push(format!("user-data-dir={}", user_data_path.display()));

        let chrome_args: Vec<String> = args.iter().map(|a| format!("--{a}")).collect();

        let mut cmd = tokio::process::Command::new(&executable);
        cmd.args(&chrome_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // On Windows: control window visibility via creation flags
        #[cfg(windows)]
        {
            if options.headless {
                cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            } else {
                cmd.creation_flags(0x00000000); // Explicitly clear to prevent auto CREATE_NO_WINDOW
            }
        }

        let child = cmd.spawn().map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!("spawn chrome: {e}")))
        })?;

        // Wait for DevToolsActivePort file (contains random port + ws path)
        let ws_url = Self::wait_for_devtools_url(&user_data_path).await?;
        tracing::info!("Chrome DevTools: {}", ws_url);

        let session = CdpSession::connect(&ws_url).await?;
        Ok(Self {
            session,
            process: child,
            user_data_path,
            _temp_dir: temp_dir,
            headless: options.headless,
        })
    }

    async fn wait_for_devtools_url(user_data_dir: &std::path::Path) -> Result<String> {
        let port_file = user_data_dir.join("DevToolsActivePort");
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);

        loop {
            if port_file.exists() {
                // Small delay to ensure file is fully written
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let content = tokio::fs::read_to_string(&port_file).await.map_err(|e| {
                    WispError::Browser(BrowserError::LaunchFailed(format!(
                        "read DevToolsActivePort: {e}"
                    )))
                })?;
                let mut lines = content.lines();
                let port = lines
                    .next()
                    .ok_or_else(|| {
                        WispError::Browser(BrowserError::LaunchFailed(
                            "empty DevToolsActivePort".into(),
                        ))
                    })?
                    .trim();
                let ws_path = lines.next().unwrap_or("/devtools/browser");
                return Ok(format!("ws://127.0.0.1:{port}{ws_path}"));
            }
            if tokio::time::Instant::now() > deadline {
                return Err(WispError::Browser(BrowserError::LaunchFailed(
                    "Chrome did not start within 15s".into(),
                )));
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
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), self.process.wait()).await;
        Ok(())
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.process.start_kill();
        // _temp_dir 的 TempDir 会在 Drop 时自动清理临时目录，无需手动 thread::spawn。
        // 用户自定义目录（_temp_dir=None）不会被清理。
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LaunchOptions;

    // === 集成测试（需要 Chrome 环境） ===
    // 运行方式：cargo test --lib browser::tests -- --ignored

    /// 测试浏览器启动和关闭。
    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn test_browser_launch_and_close() {
        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("浏览器应成功启动");

        browser.close().await.expect("浏览器应成功关闭");
    }

    /// 测试创建新页面（tab）。
    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn test_browser_new_page() {
        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("启动浏览器");

        let mut page = browser.new_page().await.expect("应成功创建页面");
        assert!(!page.session_id.is_empty(), "页面应有 session_id");

        let _ = page.close().await;
        browser.close().await.expect("关闭浏览器");
    }

    /// 测试页面导航和 JS 执行。
    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn test_page_navigation_and_evaluate() {
        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("启动浏览器");

        let mut page = browser.new_page().await.expect("创建页面");

        // 导航到 data URL
        page.goto("data:text/html,<html><body><h1>Test</h1></body></html>")
            .await
            .expect("导航应成功");

        // 执行 JS
        let result = page.evaluate("1 + 1").await.expect("JS 执行应成功");
        assert_eq!(result.as_i64(), Some(2));

        // 获取字符串
        let text = page
            .evaluate_as_string("document.querySelector('h1').textContent")
            .await
            .expect("获取文本应成功");
        assert_eq!(text, "Test");

        let _ = page.close().await;
        browser.close().await.expect("关闭浏览器");
    }

    /// 测试 CDP 命令执行。
    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn test_cdp_command() {
        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("启动浏览器");

        let mut page = browser.new_page().await.expect("创建页面");

        // 执行 CDP 命令获取浏览器版本
        let result = page.cmd("Browser.getVersion", json!({})).await;
        assert!(result.is_ok(), "Browser.getVersion 应成功");
        let version_info = result.unwrap();
        assert!(version_info.get("product").is_some(), "应包含 product 字段");

        let _ = page.close().await;
        browser.close().await.expect("关闭浏览器");
    }

    /// 测试多页面并发。
    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn test_multiple_pages() {
        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("启动浏览器");

        let mut page1 = browser.new_page().await.expect("创建页面 1");
        let mut page2 = browser.new_page().await.expect("创建页面 2");

        // 两个页面应有不同的 session_id
        assert_ne!(
            page1.session_id, page2.session_id,
            "不同页面应有不同 session_id"
        );

        let _ = page1.close().await;
        let _ = page2.close().await;
        browser.close().await.expect("关闭浏览器");
    }
}
