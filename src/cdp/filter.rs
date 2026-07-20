/// CDP methods that must NEVER be sent to avoid detection.
const BLOCKED_METHODS: &[&str] = &[
    "Runtime.enable",
    "Console.enable",
];

/// Returns true if the given CDP method should be blocked.
pub fn should_block(method: &str) -> bool {
    BLOCKED_METHODS.contains(&method)
}

/// Returns true if the given CDP method is safe to send.
pub fn is_allowed(method: &str) -> bool {
    !should_block(method)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocks_runtime_enable() {
        assert!(should_block("Runtime.enable"));
    }

    #[test]
    fn test_blocks_console_enable() {
        assert!(should_block("Console.enable"));
    }

    #[test]
    fn test_allows_runtime_evaluate() {
        assert!(!should_block("Runtime.evaluate"));
    }

    #[test]
    fn test_allows_page_navigate() {
        assert!(!should_block("Page.navigate"));
    }

    #[test]
    fn test_allows_page_create_isolated_world() {
        assert!(!should_block("Page.createIsolatedWorld"));
    }

    #[test]
    fn test_is_allowed_inverse() {
        assert!(!is_allowed("Runtime.enable"));
        assert!(is_allowed("Runtime.evaluate"));
    }
}
