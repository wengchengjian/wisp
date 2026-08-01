//! Chrome 启动参数构建。

use wisp_core::config::LaunchOptions;

/// Build default Chrome launch arguments from options, with patches applied.
/// These args include the "--" prefix (for testing/verification).
pub fn build_default_args(options: &LaunchOptions) -> Vec<String> {
    build_stealth_args(options)
        .iter()
        .map(|a| format!("--{a}"))
        .collect()
}

fn base_stealth_args() -> Vec<String> {
    vec![
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
        "disable-blink-features=AutomationControlled".into(),
        "disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,DestroyProfileOnBrowserClose,DialMediaRouteProvider,GlobalMediaControls,HttpsUpgrades,LensOverlay,MediaRouter,PaintHolding".into(),
    ]
}

fn push_proxy_arg(args: &mut Vec<String>, options: &LaunchOptions) {
    let Some(ref proxy) = options.proxy else {
        return;
    };
    args.push(format!("proxy-server={}", proxy.server));
    if proxy.username.is_some() || proxy.password.is_some() {
        tracing::warn!(
            "Browser proxy auth (username/password) is not supported via --proxy-server. \
             The proxy will be used without authentication; expect 407 responses."
        );
    }
}

fn push_extra_args(args: &mut Vec<String>, options: &LaunchOptions) {
    for arg in &options.args {
        let stripped = arg.strip_prefix("--").unwrap_or(arg);
        args.push(stripped.to_string());
    }
}

/// Build stealth launch args WITHOUT "--" prefix.
/// Aligned with patchright's chromiumSwitches.ts for maximum stealth.
pub fn build_stealth_args(options: &LaunchOptions) -> Vec<String> {
    let mut args = base_stealth_args();
    push_proxy_arg(&mut args, options);
    push_extra_args(&mut args, options);
    args
}
