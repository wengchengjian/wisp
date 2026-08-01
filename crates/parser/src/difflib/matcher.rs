//! SequenceMatcher 公开接口。

use super::SequenceMatcher;

impl<'a, T: PartialEq + std::hash::Hash + Eq> SequenceMatcher<'a, T> {
    /// Create a new matcher. autojunk defaults to true (matches Python).
    pub fn new(a: &'a [T], b: &'a [T]) -> Self {
        let mut fullbcount: std::collections::HashMap<&'a T, usize> =
            std::collections::HashMap::new();
        for elt in b {
            *fullbcount.entry(elt).or_insert(0) += 1;
        }

        let mut b2j: std::collections::HashMap<&'a T, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, elt) in b.iter().enumerate() {
            if let Some(count) = fullbcount.get(elt) {
                // autojunk threshold: > len(b)/100 + 3
                if *count <= b.len() / 100 + 3 {
                    b2j.entry(elt).or_default().push(i);
                }
            }
        }

        Self {
            a,
            b,
            autojunk: true,
            b2j,
            b_junk: None,
        }
    }

    /// Disable autojunk heuristic.
    pub fn without_autojunk(mut self) -> Self {
        self.autojunk = false;
        // Rebuild b2j without junk filtering
        let mut b2j: std::collections::HashMap<&'a T, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, elt) in self.b.iter().enumerate() {
            b2j.entry(elt).or_default().push(i);
        }
        self.b2j = b2j;
        self.b_junk = None;
        self
    }

    /// Find the longest matching block in a[a1..a2] and b[b1..b2].
    ///
    /// Returns Match { a_start, b_start, size } where size is the length of
    /// the longest common substring starting at those positions.
    pub fn find_longest_match(&self, a1: usize, a2: usize, b1: usize, b2: usize) -> super::Match {
        let mut best = (a1, b1, 0usize);
        let mut j2len: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        for i in a1..a2 {
            j2len = self.advance_j2len(i, j2len, b1, b2, &mut best);
        }
        let (besti, bestj, bestsize) = self.extend_match_backward(a1, b1, best.0, best.1, best.2);
        let bestsize = self.extend_match_forward(a2, b2, besti, bestj, bestsize);
        super::Match {
            a_start: besti,
            b_start: bestj,
            size: bestsize,
        }
    }

    /// Return similarity ratio in [0.0, 1.0]. Matches Python's ratio().
    ///
    /// ratio = 2.0 * M / T
    /// where M = sum of matching block sizes, T = len(a) + len(b)
    pub fn ratio(&self) -> f64 {
        let blocks = self.matching_blocks();
        let matches: usize = blocks.iter().map(|m| m.size).sum();
        let total = self.a.len() + self.b.len();
        if total == 0 {
            return 1.0; // empty == empty
        }
        2.0 * matches as f64 / total as f64
    }
}
