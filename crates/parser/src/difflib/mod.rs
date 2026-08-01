//! Rust port of Python's difflib.SequenceMatcher.
//!
//! Reference: https://docs.python.org/3/library/difflib.html#difflib.SequenceMatcher
//! Used by adaptive relocation to compute similarity ratios between
//! text/attribute/path sequences.

mod algorithm;
mod matcher;

/// A match block: a[a_start..a_start+size] == b[b_start..b_start+size].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    /// 序列 a 中的起始位置。
    pub a_start: usize,
    /// 序列 b 中的起始位置。
    pub b_start: usize,
    /// 匹配块大小。
    pub size: usize,
}

/// SequenceMatcher port. Computes longest matching blocks and ratio.
///
/// `autojunk`: when true, treats elements that appear > len(b)/100 + 3 times
/// in `b` as "junk" and skips them in find_longest_match. Python default is true.
pub struct SequenceMatcher<'a, T: PartialEq> {
    a: &'a [T],
    b: &'a [T],
    autojunk: bool,
    b2j: std::collections::HashMap<&'a T, Vec<usize>>,
    b_junk: Option<std::collections::HashSet<&'a T>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_test() {
        let a: Vec<char> = "abc".chars().collect();
        let b: Vec<char> = "abc".chars().collect();
        assert!((SequenceMatcher::new(&a, &b).ratio() - 1.0).abs() < 1e-9);
    }
}
