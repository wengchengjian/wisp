//! 随机值生成工具。

use rand::rngs::{SmallRng, SysRng};
use rand::{RngExt, SeedableRng};

/// 生成短随机后缀（用于唯一临时目录名等场景）。
///
/// 基于 `rand` crate 生成随机 u64 的十六进制字符串，碰撞概率极低（2^-64）。
pub fn rand_suffix() -> String {
    let mut rng = SmallRng::try_from_rng(&mut SysRng).expect("OS RNG failed");
    let val: u64 = rng.random();
    format!("{val:x}")
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

    #[test]
    fn rand_suffix_unique_across_calls() {
        let a = rand_suffix();
        let b = rand_suffix();
        assert_ne!(a, b, "连续两次调用应产生不同后缀");
    }
}
