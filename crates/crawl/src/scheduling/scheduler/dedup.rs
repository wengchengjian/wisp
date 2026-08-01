//! URL 去重策略与指纹。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// 去重策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupStrategy {
    /// 存储原始 URL（精确，内存较大）。默认选项，对 99% 场景足够。
    Exact,
    /// u64 指纹（省内存，有碰撞风险）。适合千万级 URL 大规模爬取。
    Fingerprint,
}

/// 生成 URL 的 u64 指纹（Fingerprint 去重模式使用）。
pub(super) fn fingerprint(url: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    hasher.finish()
}
