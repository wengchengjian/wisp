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

fn browser_names(options: &LaunchOptions) -> Vec<String> {
    match options.channel.as_deref() {
        Some("chrome") => vec!["chrome", "google-chrome", "google-chrome-stable"]
            .into_iter()
            .map(String::from)
            .collect(),
        Some("msedge") => vec!["msedge", "microsoft-edge"]
            .into_iter()
            .map(String::from)
            .collect(),
        Some("chromium") => vec!["chromium", "chromium-browser"]
            .into_iter()
            .map(String::from)
            .collect(),
        None => vec![
            "chrome",
            "google-chrome",
            "chromium",
            "chromium-browser",
            "msedge",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        Some(other) => vec![other.to_string()],
    }
}

fn find_windows_browser() -> Option<PathBuf> {
    if !cfg!(target_os = "windows") {
        return None;
    }
    let windows_paths = [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ];
    windows_paths.iter().map(PathBuf::from).find(|p| p.exists())
}

fn find_in_path(names: &[String]) -> Option<PathBuf> {
    names.iter().find_map(|name| which::which(name).ok())
}

/// Resolve the browser executable path from options.
///
/// 查找顺序：
/// 1. `options.executable_path`（用户显式指定，信任其版本）
/// 2. **自动下载/使用 Chrome for Testing**（wreq 支持的版本，保证 TLS 指纹与 HTTP
///    快速路径一致）。不再主动探测本地浏览器——因为 `chrome --version` 探测在 Windows
///    上会**拉起本地浏览器进程**（副作用，用户会看到多余浏览器窗口），且本地 Chrome
///    多为最新版（>wreq 可模拟上限），指纹不自洽。下载版本失败时才回退本地浏览器。
pub async fn resolve_executable(options: &LaunchOptions) -> Result<PathBuf> {
    if let Some(path) = resolve_explicit_path(options)? {
        return Ok(path);
    }

    // 优先使用下载的 Chrome for Testing（wreq 支持版本）
    match crate::installer::ensure_browser_installed().await {
        Ok(path) => Ok(path),
        Err(e) => {
            // 下载失败（如网络不可用）回退到本地浏览器，但不做版本探测（避免拉起进程）
            tracing::warn!("下载 Chrome for Testing 失败: {e}，回退到本地浏览器");
            let names = browser_names(options);
            if let Some(path) = find_windows_browser() {
                return Ok(path);
            }
            if let Some(path) = find_in_path(&names) {
                return Ok(path);
            }
            Err(e)
        }
    }
}
