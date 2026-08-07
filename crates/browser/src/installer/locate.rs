//! Chrome 可执行文件定位与平台标识。

use std::path::{Path, PathBuf};

use wisp_core::error::Result;

pub(crate) fn get_install_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".browsers")
}

/// 在安装目录中查找已安装的浏览器
#[allow(dead_code)] // 仅测试中使用
pub(crate) fn find_installed_browser(install_root: &Path) -> Result<Option<PathBuf>> {
    if !install_root.exists() {
        return Ok(None);
    }

    // 遍历 .browsers/chrome-*/ 目录
    for entry in std::fs::read_dir(install_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir()
            && let Some(exe) = find_executable_in_dir(&path)?
        {
            return Ok(Some(exe));
        }
    }
    Ok(None)
}

/// 在安装目录中查找指定主版本的已安装浏览器。
///
/// 遍历 `.browsers/chrome-{full_version}/`，解析目录名中的主版本号（第一个点号前），
/// 与 `major` 匹配则返回其可执行文件路径。用于复用已下载的指定版本（如 132/147），
/// 避免重新下载该主版本下的其他小版本。
pub(crate) fn find_installed_browser_for_major(
    install_root: &Path,
    major: u32,
) -> Result<Option<PathBuf>> {
    if !install_root.exists() {
        return Ok(None);
    }
    for entry in std::fs::read_dir(install_root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // 目录名形如 `chrome-132.0.6834.159`
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(maj_str) = name
            .strip_prefix("chrome-")
            .and_then(|s| s.split('.').next())
        else {
            continue;
        };
        let Ok(dir_major) = maj_str.parse::<u32>() else {
            continue;
        };
        if dir_major != major {
            continue;
        }
        if let Some(exe) = find_executable_in_dir(&path)? {
            return Ok(Some(exe));
        }
    }
    Ok(None)
}

/// 在版本目录中查找可执行文件
pub(crate) fn find_executable_in_dir(version_dir: &Path) -> Result<Option<PathBuf>> {
    let platform = get_platform_id();
    let chrome_dir = version_dir.join(format!("chrome-{platform}"));

    let exe_rel = if cfg!(target_os = "windows") {
        "chrome.exe"
    } else if cfg!(target_os = "macos") {
        "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"
    } else {
        "chrome"
    };

    let exe_path = chrome_dir.join(exe_rel);
    if exe_path.exists() {
        return Ok(Some(exe_path));
    }

    Ok(None)
}

/// 获取当前平台的标识
pub fn get_platform_id() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "linux64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "mac-x64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "mac-arm64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "win64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86"))]
    {
        "win32"
    }
}
