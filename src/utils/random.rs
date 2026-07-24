//! 随机值生成工具。

/// 生成短随机后缀（用于唯一临时目录名等场景）。
///
/// 基于当前时间纳秒分量生成十六进制字符串。
pub fn rand_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:x}", nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rand_suffix_non_empty() {
        let s = rand_suffix();
        assert!(!s.is_empty());
    }

    #[test]
    fn rand_suffix_is_hex() {
        let s = rand_suffix();
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
