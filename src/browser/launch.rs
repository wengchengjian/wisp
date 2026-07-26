use std::path::PathBuf;

use crate::config::LaunchOptions;
use crate::error::{BrowserError, Result, WispError};

/// Resolve the browser executable path from options.
///
/// 查找顺序：
/// 1. `options.executable_path`（用户显式指定）
/// 2. Windows 常见安装路径
/// 3. `which::which` 查找系统 PATH 中的浏览器
/// 4. **自动下载安装 Chrome for Testing**（若以上都失败）
pub async fn resolve_executable(options: &LaunchOptions) -> Result<PathBuf> {
    if let Some(ref path) = options.executable_path {
        if path.exists() {
            return Ok(path.clone());
        }
        return Err(WispError::Browser(BrowserError::LaunchFailed(format!(
            "Executable not found: {}",
            path.display()
        ))));
    }

    let names: Vec<&str> = match options.channel.as_deref() {
        Some("chrome") => vec!["chrome", "google-chrome", "google-chrome-stable"],
        Some("msedge") => vec!["msedge", "microsoft-edge"],
        Some("chromium") => vec!["chromium", "chromium-browser"],
        None => vec![
            "chrome",
            "google-chrome",
            "chromium",
            "chromium-browser",
            "msedge",
        ],
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

    // 以上都失败：自动下载安装 Chrome for Testing
    // （ensure_browser_installed 内部会先检查已安装，未安装时才下载）
    super::installer::ensure_browser_installed().await
}

/// Build default Chrome launch arguments from options, with patches applied.
/// These args include the "--" prefix (for testing/verification).
///
/// ARCH: = build_common_args + build_stealth_extra_args，加 "--" 前缀。
#[must_use]
pub fn build_default_args(options: &LaunchOptions) -> Vec<String> {
    build_stealth_args(options)
        .iter()
        .map(|a| format!("--{a}"))
        .collect()
}

/// 构建完整启动参数（common + stealth_extra），无 "--" 前缀。
///
/// ARCH: 保留原 `build_stealth_args` 名字以兼容 `Browser::launch` 调用。
#[allow(unused_mut)]
pub fn build_stealth_args(options: &LaunchOptions) -> Vec<String> {
    let mut args = build_common_args(options);
    #[cfg(feature = "stealth")]
    {
        args.extend(build_stealth_extra_args(options));
    }
    args
}

/// 构建通用 Chrome 启动参数（不含 stealth 专用参数）。
///
/// ARCH: 通用参数对所有浏览器模式（Dynamic/Stealth）都需要。
/// 不包含 `disable-blink-features=AutomationControlled` 等 stealth 专用参数。
pub fn build_common_args(options: &LaunchOptions) -> Vec<String> {
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
        // Disabled features (from patchright)
        "disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,DestroyProfileOnBrowserClose,DialMediaRouteProvider,GlobalMediaControls,HttpsUpgrades,LensOverlay,MediaRouter,PaintHolding".into(),
    ];

    // NOTE: Do NOT add --no-sandbox (causes Chrome re-launch on Windows headed mode)
    // NOTE: Do NOT add --enable-automation, --disable-popup-blocking, etc.

    // Proxy
    if let Some(ref proxy) = options.proxy {
        args.push(format!("proxy-server={}", proxy.server));
        // Chrome --proxy-server 不支持内联认证；username/password 无法通过命令行传递。
        // 需通过 CDP Fetch.requestPaused 拦截 407 或扩展程序处理（当前未实现）。
        if proxy.username.is_some() || proxy.password.is_some() {
            tracing::warn!(
                "Browser proxy auth (username/password) is not supported via --proxy-server. \
                 The proxy will be used without authentication; expect 407 responses. \
                 To use authenticated proxies with browser mode, configure the proxy to \
                 whitelist the client IP or use an unauthenticated proxy."
            );
        }
    }

    // User-provided extra args (strip -- prefix if present)
    for arg in &options.args {
        let stripped = arg.strip_prefix("--").unwrap_or(arg);
        args.push(stripped.to_string());
    }

    args
}

/// 构建 stealth 专用额外参数。
///
/// ARCH: 仅在 stealth feature 启用时编译。browser-only 模式下不包含反检测参数。
#[cfg(feature = "stealth")]
pub fn build_stealth_extra_args(_options: &LaunchOptions) -> Vec<String> {
    vec![
        // Core anti-detection flag
        "disable-blink-features=AutomationControlled".into(),
    ]
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
        #[cfg(feature = "stealth")]
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
    fn test_stealth_args_proxy_with_auth_still_sets_server() {
        let opts = LaunchOptions {
            proxy: Some(crate::config::ProxyConfig {
                server: "http://127.0.0.1:8080".into(),
                username: Some("user".into()),
                password: Some("pass".into()),
            }),
            ..Default::default()
        };
        let args = build_stealth_args(&opts);
        // proxy-server 仍设置
        assert!(
            args.iter()
                .any(|a| a == "proxy-server=http://127.0.0.1:8080"),
            "proxy-server 应设置"
        );
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

    #[test]
    fn test_build_common_args_excludes_stealth_specific() {
        let opts = LaunchOptions::default();
        let args = build_common_args(&opts);
        // 通用参数应包含 no-first-run、disable-background-networking
        assert!(args.contains(&"no-first-run".to_string()));
        assert!(args.contains(&"disable-background-networking".to_string()));
        // 通用参数不应包含 stealth 专用参数
        assert!(!args.contains(&"disable-blink-features=AutomationControlled".to_string()));
    }

    #[cfg(feature = "stealth")]
    #[test]
    fn test_build_stealth_extra_args_contains_stealth_specific() {
        let opts = LaunchOptions::default();
        let args = build_stealth_extra_args(&opts);
        assert!(args.contains(&"disable-blink-features=AutomationControlled".to_string()));
        // stealth_extra 不应包含通用参数
        assert!(!args.contains(&"no-first-run".to_string()));
    }

    #[cfg(feature = "stealth")]
    #[test]
    fn test_build_default_args_combines_common_and_stealth() {
        let opts = LaunchOptions::default();
        let args = build_default_args(&opts);
        // 应同时包含通用参数和 stealth 专用参数
        assert!(args.iter().all(|a| a.starts_with("--")));
        assert!(args.iter().any(|a| a == "--no-first-run"));
        assert!(args.iter().any(|a| a == "--disable-blink-features=AutomationControlled"));
    }

    #[cfg(feature = "stealth")]
    #[test]
    fn test_build_stealth_args_equals_common_plus_stealth_extra() {
        let opts = LaunchOptions::default();
        let common = build_common_args(&opts);
        let extra = build_stealth_extra_args(&opts);
        let combined = build_stealth_args(&opts);
        // combined 应等于 common + extra（顺序可能不同，但内容相同）
        let mut expected = common.clone();
        expected.extend(extra.clone());
        expected.sort();
        let mut actual = combined.clone();
        actual.sort();
        assert_eq!(expected, actual);
    }
}
