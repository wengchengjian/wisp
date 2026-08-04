//! Browser process management. Launches Chrome directly with stealth args.

/// CDP WebSocket 会话管理。
pub mod cdp;
/// Browser cookie jar（CDP 实现）。
pub mod cookie;
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
/// 反检测引擎（CF 挑战 / Turnstile / 人类行为 / Stealth 策略）。
pub mod stealth;
/// 浏览器抓取策略 trait（领域契约）。
pub mod strategy;
/// 浏览器页面响应提取。
pub mod extract;

mod process;

pub use cdp::CdpSession;
pub use cookie::BrowserCookieJar;
pub use page::Page;
pub use pool::BrowserPool;
pub use stealth::StealthStrategy;
pub use strategy::BrowserFetchStrategy;

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use wisp_core::config::LaunchOptions;
use wisp_core::error::Result;

/// 浏览器实例：封装 Chrome 进程 + CDP 会话。
///
/// 通过 [`Browser::launch`] 启动，使用 [`Browser::new_page`] 创建页面，
/// 最后调用 [`Browser::close`] 关闭。
pub struct Browser {
    session: Arc<CdpSession>,
    process: tokio::process::Child,
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
        let (user_data_path, temp_dir) = process::setup_user_data_dir(&options)?;
        let port_file = user_data_path.join("DevToolsActivePort");
        let _ = std::fs::remove_file(&port_file);
        let chrome_args = process::build_chrome_args(&options, &user_data_path)?;
        let child = process::spawn_chrome_process(&executable, &chrome_args, options.headless)?;
        let ws_url = process::wait_for_devtools_url(&user_data_path).await?;
        tracing::info!("Chrome DevTools: {}", ws_url);
        let session = CdpSession::connect(&ws_url).await?;
        Ok(Self {
            session,
            process: child,
            _temp_dir: temp_dir,
            headless: options.headless,
        })
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
        // 先等优雅退出；超时才 kill 整个 Chrome 进程树，避免子进程仍占用临时目录。
        let exited =
            process::wait_for_process_exit(&mut self.process, Duration::from_secs(3)).await;
        if !exited {
            if let Some(pid) = self.process.id() {
                process::kill_process_tree(pid);
            }
            let _ = process::wait_for_process_exit(&mut self.process, Duration::from_secs(3)).await;
        }
        Ok(())
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Drop 无法 await：先杀进程树并短等退出，保证 TempDir 删除时文件未被占用。
        if let Some(pid) = self.process.id() {
            process::kill_process_tree(pid);
        }
        let _ = self.process.start_kill();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if matches!(self.process.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        // _temp_dir 的 TempDir 会在 Drop 时自动删除临时目录，无需手动 thread::spawn。
        // 用户自定义目录（_temp_dir=None）不会被清理。
    }
}

#[cfg(test)]
mod tests;
