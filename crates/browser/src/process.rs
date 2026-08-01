//! Chrome 进程启动与 DevTools 端口读取。

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::process::Child;

use wisp_core::config::LaunchOptions;
use wisp_core::error::{BrowserError, Result, WispError};

use crate::launch;

/// 清理已退出 wisp 进程遗留的临时 Chrome profile。
///
/// 目录名形如 `wisp-{pid}-{suffix}`；只删除 PID 已不存在的目录，
/// 避免误删其他正在运行的 wisp 实例。
pub(super) fn cleanup_stale_temp_dirs() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(pid) = parse_wisp_temp_pid(name) else {
            continue;
        };
        if pid == std::process::id() || process_alive(pid) {
            continue;
        }
        let _ = std::fs::remove_dir_all(entry.path());
    }
}

fn parse_wisp_temp_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("wisp-")?;
    let pid = rest.split('-').next()?;
    pid.parse().ok()
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    let query = format!("PID eq {pid}");
    match std::process::Command::new("tasklist")
        .args(["/FI", &query, "/NH"])
        .output()
    {
        Ok(out) => String::from_utf8_lossy(&out.stdout).contains(&pid.to_string()),
        Err(_) => true,
    }
}

#[cfg(not(windows))]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(true)
}

pub(super) fn setup_user_data_dir(
    options: &LaunchOptions,
) -> Result<(PathBuf, Option<tempfile::TempDir>)> {
    if let Some(ref dir) = options.user_data_dir {
        return Ok((dir.clone(), None));
    }
    cleanup_stale_temp_dirs();
    let td = tempfile::Builder::new()
        .prefix(&format!("wisp-{}-", std::process::id()))
        .tempdir()
        .map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!("create temp dir: {e}")))
        })?;
    Ok((td.path().to_path_buf(), Some(td)))
}

pub(super) fn build_chrome_args(
    options: &LaunchOptions,
    user_data_path: &std::path::Path,
) -> Vec<String> {
    let mut args = launch::build_stealth_args(options);
    // 临时 profile 会触发组件下载（AI 模型、优化提示等），爬虫场景不需要它们。
    args.push("disable-component-update".to_string());
    args.push("remote-debugging-port=0".to_string());
    if options.headless {
        args.push("headless=new".to_string());
    }
    args.push(format!("user-data-dir={}", user_data_path.display()));
    args.iter().map(|a| format!("--{a}")).collect()
}

pub(super) fn spawn_chrome_process(
    executable: &std::path::Path,
    args: &[String],
    headless: bool,
) -> Result<Child> {
    let mut cmd = tokio::process::Command::new(executable);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        if headless {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        } else {
            cmd.creation_flags(0x00000000);
        }
    }
    cmd.spawn()
        .map_err(|e| WispError::Browser(BrowserError::LaunchFailed(format!("spawn chrome: {e}"))))
}

#[cfg(windows)]
pub(super) fn kill_process_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(windows))]
pub(super) fn kill_process_tree(pid: u32) {
    let _ = std::process::Command::new("pkill")
        .args(["-TERM", "-P", &pid.to_string()])
        .status();
    let _ = std::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status();
}

/// 等待子进程退出；超时返回 `false`（不消费 `Child`，失败后可继续 kill）。
pub(super) async fn wait_for_process_exit(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(super) async fn wait_for_devtools_url(user_data_dir: &std::path::Path) -> Result<String> {
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
            return Ok(format!("ws://127.0.0.1:{}{}", port, ws_path));
        }
        if tokio::time::Instant::now() > deadline {
            return Err(WispError::Browser(BrowserError::LaunchFailed(
                "Chrome did not start within 15s".into(),
            )));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
