//! Chrome for Testing zip 解压与权限修复。

use std::path::{Path, PathBuf};

use super::locate::find_executable_in_dir;
use wisp_core::error::{BrowserError, Result, WispError};

pub(super) fn extract_and_locate(
    version: &str,
    install_root: &Path,
    temp_zip: &Path,
) -> Result<PathBuf> {
    let version_dir = install_root.join(format!("chrome-{version}"));
    tracing::info!("解压到: {}", version_dir.display());
    extract_zip(temp_zip, &version_dir)?;
    let _ = std::fs::remove_file(temp_zip);
    find_executable_in_dir(&version_dir)?.ok_or_else(|| {
        WispError::Browser(BrowserError::LaunchFailed(
            "解压后未找到 Chrome 可执行文件".into(),
        ))
    })
}

/// 递归设置目录下所有文件的可执行权限（0o755）。
///
/// Chrome 目录下的可执行文件（chrome、chrome_crashpad_handler）和共享库（.so）
/// 都需要可执行权限。zip 解压可能不保留原始权限，需要手动修复。
#[cfg(unix)]
pub(super) fn fix_permissions(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fn walk(dir: &Path) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(
                    |e| {
                        WispError::Browser(BrowserError::LaunchFailed(format!(
                            "设置目录权限失败: {e}"
                        )))
                    },
                )?;
                walk(&path)?;
            } else {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(
                    |e| {
                        WispError::Browser(BrowserError::LaunchFailed(format!(
                            "设置文件权限失败: {e}"
                        )))
                    },
                )?;
            }
        }
        Ok(())
    }

    walk(dir)
}

fn extract_zip_entry(
    archive: &mut zip::ZipArchive<std::fs::File>,
    index: usize,
    dest: &Path,
) -> Result<()> {
    let mut entry = archive.by_index(index).map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!(
            "读取 zip 条目 {index} 失败: {e}"
        )))
    })?;
    let name = entry.name().to_string();
    let outpath = dest.join(&name);
    if entry.is_dir() {
        std::fs::create_dir_all(&outpath).map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!("创建目录失败: {e}")))
        })?;
        return Ok(());
    }
    if let Some(parent) = outpath.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!("创建父目录失败: {e}")))
        })?;
    }
    let mut outfile = std::fs::File::create(&outpath).map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!("创建文件失败: {e}")))
    })?;
    std::io::copy(&mut entry, &mut outfile).map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!("写入文件失败: {e}")))
    })?;
    drop(outfile);
    #[cfg(unix)]
    {
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

/// 解压 zip 文件到目标目录
fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path).map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!(
            "打开 zip 文件失败: {e}"
        )))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!(
            "读取 zip 归档失败: {e}"
        )))
    })?;
    for i in 0..archive.len() {
        extract_zip_entry(&mut archive, i, dest)?;
    }
    Ok(())
}
