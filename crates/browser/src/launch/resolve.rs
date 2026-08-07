//! 浏览器可执行文件查找。

use std::path::PathBuf;

use wisp_core::config::LaunchOptions;
use wisp_core::error::{BrowserError, Result, WispError};

fn resolve_explicit_path(options: &LaunchOptions) -> Result<Option<PathBuf>> {
    let Some(path) = options.executable_path.as_ref() else {
        return Ok(None);
    };
    if path.exists() {
        return Ok(Some(path.clone()));
    }
    Err(WispError::Browser(BrowserError::LaunchFailed(format!(
        "Executable not found: {}",
        path.display()
    ))))
}

/// Resolve the browser executable path from options.
///
/// 查找顺序：
/// 1. `options.executable_path`（用户显式指定，信任其版本）
/// 2. **固定使用下载的 Chrome for Testing 148**（与 CF 快速路径 TLS 指纹档位
///    `Chrome148` 对齐，保证浏览器与 HTTP 指纹一致）。**不使用本机 Chrome**——
///    本机浏览器版本不可控（多为最新版），指纹不自洽会被 CF 拒；且探测会拉起
///    多余浏览器窗口。下载失败（如网络不可用）直接报错，不静默回退。
pub async fn resolve_executable(options: &LaunchOptions) -> Result<PathBuf> {
    if let Some(path) = resolve_explicit_path(options)? {
        return Ok(path);
    }

    // 固定使用下载的 Chrome for Testing 148
    crate::installer::ensure_browser_installed().await
}
