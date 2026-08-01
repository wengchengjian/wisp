use super::*;
use wisp_core::config::LaunchOptions;

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
        proxy: Some(wisp_core::config::ProxyConfig {
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
        proxy: Some(wisp_core::config::ProxyConfig {
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
