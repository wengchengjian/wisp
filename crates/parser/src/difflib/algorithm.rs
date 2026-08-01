//! SequenceMatcher 内部匹配算法。

use std::collections::HashMap;

use super::Match;

impl<'a, T: PartialEq + std::hash::Hash + Eq> super::SequenceMatcher<'a, T> {
    pub(super) fn advance_j2len(
        &self,
        i: usize,
        j2len: HashMap<usize, usize>,
        b1: usize,
        b2: usize,
        best: &mut (usize, usize, usize),
    ) -> HashMap<usize, usize> {
        let mut newj2len: HashMap<usize, usize> = HashMap::new();
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
                if k > best.2 {
                    *best = (i + 1 - k, j + 1 - k, k);
                }
            }
        }
        newj2len
    }

    pub(super) fn extend_match_backward(
        &self,
        a1: usize,
        b1: usize,
        besti: usize,
        bestj: usize,
        bestsize: usize,
    ) -> (usize, usize, usize) {
        let mut besti = besti;
        let mut bestj = bestj;
        let mut bestsize = bestsize;
        while besti > a1
            && bestj > b1
            && self.a[besti - 1] == self.b[bestj - 1]
            && !self.is_junk_at(bestj - 1)
        {
            besti -= 1;
            bestj -= 1;
            bestsize += 1;
        }
        (besti, bestj, bestsize)
    }

    pub(super) fn extend_match_forward(
        &self,
        a2: usize,
        b2: usize,
        besti: usize,
        bestj: usize,
        bestsize: usize,
    ) -> usize {
        let mut bestsize = bestsize;
        while besti + bestsize < a2
            && bestj + bestsize < b2
            && self.a[besti + bestsize] == self.b[bestj + bestsize]
            && !self.is_junk_at(bestj + bestsize)
        {
            bestsize += 1;
        }
        bestsize
    }

    fn is_junk_at(&self, j: usize) -> bool {
        match &self.b_junk {
            Some(junk) => junk.contains(&self.b[j]),
            None => false,
        }
    }

    /// Compute matching blocks (vector of non-overlapping Match, last is always {0,0,0}).
    pub(super) fn matching_blocks(&self) -> Vec<Match> {
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
        blocks.push(Match {
            a_start: la,
            b_start: lb,
            size: 0,
        });
        blocks
    }
}
