//! Rust port of Python's difflib.SequenceMatcher.
//!
//! Reference: https://docs.python.org/3/library/difflib.html#difflib.SequenceMatcher
//! Used by adaptive relocation to compute similarity ratios between
//! text/attribute/path sequences.

/// A match block: a[a_start..a_start+size] == b[b_start..b_start+size].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub a_start: usize,
    pub b_start: usize,
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
    fullbcount: std::collections::HashMap<&'a T, usize>,
    b_junk: Option<std::collections::HashSet<&'a T>>,
}

impl<'a, T: PartialEq + std::hash::Hash + Eq> SequenceMatcher<'a, T> {
    /// Create a new matcher. autojunk defaults to true (matches Python).
    pub fn new(a: &'a [T], b: &'a [T]) -> Self {
        let mut fullbcount: std::collections::HashMap<&'a T, usize> = std::collections::HashMap::new();
        for elt in b {
            *fullbcount.entry(elt).or_insert(0) += 1;
        }

        let mut b2j: std::collections::HashMap<&'a T, Vec<usize>> = std::collections::HashMap::new();
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
            fullbcount,
            b_junk: None,
        }
    }

    /// Disable autojunk heuristic.
    pub fn without_autojunk(mut self) -> Self {
        self.autojunk = false;
        // Rebuild b2j without junk filtering
        let mut b2j: std::collections::HashMap<&'a T, Vec<usize>> = std::collections::HashMap::new();
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
    pub fn find_longest_match(&self, a1: usize, a2: usize, b1: usize, b2: usize) -> Match {
        let mut besti = a1;
        let mut bestj = b1;
        let mut bestsize: usize = 0;

        // j2len[j] = length of longest match ending with a[i-1] and b[j]
        let mut j2len: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();

        for i in a1..a2 {
            let mut newj2len: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            if let Some(indices) = self.b2j.get(&self.a[i]) {
                for &j in indices {
                    if j < b1 {
                        continue;
                    }
                    if j >= b2 {
                        break;
                    }
                    let k = if j > 0 {
                        j2len.get(&(j - 1)).copied().unwrap_or(0) + 1
                    } else {
                        1
                    };
                    newj2len.insert(j, k);
                    if k > bestsize {
                        besti = i + 1 - k;
                        bestj = j + 1 - k;
                        bestsize = k;
                    }
                }
            }
            j2len = newj2len;
        }

        // Extend match at the ends (skip junk)
        while besti > a1
            && bestj > b1
            && self.a[besti - 1] == self.b[bestj - 1]
            && !self.is_junk_at(bestj - 1)
        {
            besti -= 1;
            bestj -= 1;
            bestsize += 1;
        }
        while besti + bestsize < a2
            && bestj + bestsize < b2
            && self.a[besti + bestsize] == self.b[bestj + bestsize]
            && !self.is_junk_at(bestj + bestsize)
        {
            bestsize += 1;
        }

        Match { a_start: besti, b_start: bestj, size: bestsize }
    }

    fn is_junk_at(&self, j: usize) -> bool {
        match &self.b_junk {
            Some(junk) => junk.contains(&self.b[j]),
            None => false,
        }
    }

    /// Compute matching blocks (vector of non-overlapping Match, last is always {0,0,0}).
    fn matching_blocks(&self) -> Vec<Match> {
        let mut blocks: Vec<Match> = Vec::new();
        let la = self.a.len();
        let lb = self.b.len();

        // Stack of (a1, a2, b1, b2) ranges to process
        let mut stack: Vec<(usize, usize, usize, usize)> = vec![(0, la, 0, lb)];

        while let Some((a1, a2, b1, b2)) = stack.pop() {
            let m = self.find_longest_match(a1, a2, b1, b2);
            if m.size > 0 {
                if a1 < m.a_start && b1 < m.b_start {
                    stack.push((a1, m.a_start, b1, m.b_start));
                }
                let ma_end = m.a_start + m.size;
                let mb_end = m.b_start + m.size;
                if ma_end < a2 && mb_end < b2 {
                    stack.push((ma_end, a2, mb_end, b2));
                }
                blocks.push(m);
            }
        }

        // Sort by a_start to match Python order
        blocks.sort_by_key(|m| m.a_start);
        blocks.push(Match { a_start: la, b_start: lb, size: 0 });
        blocks
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
            return 1.0;  // empty == empty
        }
        2.0 * matches as f64 / total as f64
    }
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
