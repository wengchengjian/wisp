use std::path::PathBuf;

use crate::config::LaunchOptions;
use crate::error::{PatchrightError, Result};
use crate::patches;

/// Resolve the browser executable path from options.
pub fn resolve_executable(options: &LaunchOptions) -> Result<PathBuf> {
    if let Some(ref path) = options.executable_path {
        if path.exists() {
            return Ok(path.clone());
        }
        return Err(PatchrightError::LaunchFailed(format!(
            "Executable not found: {}",
            path.display()
        )));
    }

    let names: Vec<&str> = match options.channel.as_deref() {
        Some("chrome") => vec!["chrome", "google-chrome", "google-chrome-stable"],
        Some("msedge") => vec!["msedge", "microsoft-edge"],
        Some("chromium") => vec!["chromium", "chromium-browser"],
        None => vec!["chrome", "google-chrome", "chromium", "chromium-browser", "msedge"],
        Some(other) => vec![other],
    };

    // Try well-known Windows paths
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

    for name in &names {
        if let Ok(path) = which::which(name) {
            return Ok(path);
        }
    }

    Err(PatchrightError::LaunchFailed(
        "No Chromium-based browser found. Install Chrome/Chromium/Edge or set executable_path.".into(),
    ))
}

/// Build default Chrome launch arguments from options, with patches applied.
pub fn build_default_args(options: &LaunchOptions) -> Vec<String> {
    let mut args = Vec::new();

    if options.headless {
        args.push("--headless=new".to_string());
    }

    if !options.no_viewport {
        args.push("--window-size=1280,720".to_string());
    }

    if let Some(ref user_data_dir) = options.user_data_dir {
        args.push(format!("--user-data-dir={}", user_data_dir.display()));
    }

    if let Some(ref proxy) = options.proxy {
        args.push(format!("--proxy-server={}", proxy.server));
    }

    args.push("--no-first-run".to_string());
    args.push("--no-default-browser-check".to_string());
    args.push("--disable-background-networking".to_string());
    args.push("--disable-sync".to_string());
    args.push("--disable-translate".to_string());
    args.push("--metrics-recording-only".to_string());
    args.push("--safebrowsing-disable-auto-update".to_string());

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
        let opts = LaunchOptions { headless: true, ..Default::default() };
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
