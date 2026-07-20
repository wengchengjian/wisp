/// Flags that leak automation identity and must be removed.
const REMOVE_ARGS: &[&str] = &[
    "--enable-automation",
    "--disable-popup-blocking",
    "--disable-component-update",
    "--disable-default-apps",
    "--disable-extensions",
];

/// Flags that must be added for stealth.
const ADD_ARGS: &[&str] = &[
    "--disable-blink-features=AutomationControlled",
];

/// Patch browser launch arguments to remove detection vectors.
/// Removes automation-revealing flags and adds stealth flags.
pub fn patch_launch_args(args: &mut Vec<String>) {
    args.retain(|a| !REMOVE_ARGS.contains(&a.as_str()));
    for arg in ADD_ARGS {
        if !args.iter().any(|a| a == arg) {
            args.push(arg.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_removes_automation_flags() {
        let mut args = vec![
            "--enable-automation".to_string(),
            "--disable-popup-blocking".to_string(),
            "--disable-component-update".to_string(),
            "--disable-default-apps".to_string(),
            "--disable-extensions".to_string(),
            "--no-first-run".to_string(),
        ];
        patch_launch_args(&mut args);
        assert!(!args.contains(&"--enable-automation".to_string()));
        assert!(!args.contains(&"--disable-popup-blocking".to_string()));
        assert!(!args.contains(&"--disable-component-update".to_string()));
        assert!(!args.contains(&"--disable-default-apps".to_string()));
        assert!(!args.contains(&"--disable-extensions".to_string()));
        assert!(args.contains(&"--no-first-run".to_string()));
    }

    #[test]
    fn test_adds_stealth_flags() {
        let mut args = vec!["--no-first-run".to_string()];
        patch_launch_args(&mut args);
        assert!(args.contains(&"--disable-blink-features=AutomationControlled".to_string()));
    }

    #[test]
    fn test_no_duplicate_stealth_flags() {
        let mut args = vec![
            "--disable-blink-features=AutomationControlled".to_string(),
        ];
        patch_launch_args(&mut args);
        let count = args.iter()
            .filter(|a| *a == "--disable-blink-features=AutomationControlled")
            .count();
        assert_eq!(count, 1);
    }
}
