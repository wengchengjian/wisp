use std::path::PathBuf;

use crate::config::LaunchOptions;
use crate::error::{WispError, Result};

/// Resolve the browser executable path from options.
pub fn resolve_executable(options: &LaunchOptions) -> Result<PathBuf> {
    if let Some(ref path) = options.executable_path {
        if path.exists() {
            return Ok(path.clone());
        }
        return Err(WispError::LaunchFailed(format!(
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

    Err(WispError::LaunchFailed(
        "No Chromium-based browser found. Install Chrome/Chromium/Edge or set executable_path.".into(),
    ))
}

/// Build default Chrome launch arguments from options, with patches applied.
/// These args include the "--" prefix (for testing/verification).
pub fn build_default_args(options: &LaunchOptions) -> Vec<String> {
    build_stealth_args(options)
        .iter()
        .map(|a| format!("--{a}"))
        .collect()
}

/// Build stealth launch args WITHOUT "--" prefix.
/// Aligned with patchright's chromiumSwitches.ts for maximum stealth.
pub fn build_stealth_args(options: &LaunchOptions) -> Vec<String> {
    let mut args: Vec<String> = vec![
        // patchright chromiumSwitches (verified from source)
        "disable-field-trial-config".into(),
        "disable-background-networking".into(),
        "disable-background-timer-throttling".into(),
        "disable-backgrounding-occluded-windows".into(),
        "disable-breakpad".into(),
        "no-default-browser-check".into(),
        "disable-dev-shm-usage".into(),
        "disable-hang-monitor".into(),
        "disable-prompt-on-repost".into(),
        "disable-renderer-backgrounding".into(),
        "force-color-profile=srgb".into(),
        "no-first-run".into(),
        "password-store=basic".into(),
        "use-mock-keychain".into(),
        "no-service-autorun".into(),
        "export-tagged-pdf".into(),
        "disable-search-engine-choice-screen".into(),
        "disable-infobars".into(),
        "disable-sync".into(),
        // Core anti-detection flag
        "disable-blink-features=AutomationControlled".into(),
        // Disabled features (from patchright)
        "disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,DestroyProfileOnBrowserClose,DialMediaRouteProvider,GlobalMediaControls,HttpsUpgrades,LensOverlay,MediaRouter,PaintHolding".into(),
    ];

    // NOTE: Do NOT add --no-sandbox (causes Chrome re-launch on Windows headed mode)
    // NOTE: Do NOT add --enable-automation, --disable-popup-blocking, etc.

    // Proxy
    if let Some(ref proxy) = options.proxy {
        args.push(format!("proxy-server={}", proxy.server));
    }

    // User-provided extra args (strip -- prefix if present)
    for arg in &options.args {
        let stripped = arg.strip_prefix("--").unwrap_or(arg);
        args.push(stripped.to_string());
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stealth_args_no_automation() {
        let opts = LaunchOptions::default();
        let args = build_stealth_args(&opts);
        assert!(!args.contains(&"enable-automation".to_string()));
        assert!(!args.contains(&"disable-popup-blocking".to_string()));
        assert!(!args.contains(&"disable-component-update".to_string()));
        assert!(!args.contains(&"disable-default-apps".to_string()));
        assert!(!args.contains(&"disable-extensions".to_string()));
    }

    #[test]
    fn test_stealth_args_has_safe_defaults() {
        let opts = LaunchOptions::default();
        let args = build_stealth_args(&opts);
        assert!(args.contains(&"no-first-run".to_string()));
        assert!(args.contains(&"disable-background-networking".to_string()));
        assert!(args.contains(&"disable-blink-features=AutomationControlled".to_string()));
    }

    #[test]
    fn test_build_default_args_has_prefix() {
        let opts = LaunchOptions::default();
        let args = build_default_args(&opts);
        assert!(args.iter().all(|a| a.starts_with("--")));
        assert!(!args.contains(&"--enable-automation".to_string()));
    }

    #[test]
    fn test_stealth_args_proxy() {
        let opts = LaunchOptions {
            proxy: Some(crate::config::ProxyConfig {
                server: "http://127.0.0.1:8080".into(),
                username: None,
                password: None,
            }),
            ..Default::default()
        };
        let args = build_stealth_args(&opts);
        assert!(args.contains(&"proxy-server=http://127.0.0.1:8080".to_string()));
    }

    #[test]
    fn test_stealth_args_user_extra_args() {
        let opts = LaunchOptions {
            args: vec!["--custom-flag".to_string(), "another-flag".to_string()],
            ..Default::default()
        };
        let args = build_stealth_args(&opts);
        assert!(args.contains(&"custom-flag".to_string()));
        assert!(args.contains(&"another-flag".to_string()));
    }
}
