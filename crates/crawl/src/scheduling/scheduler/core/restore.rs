//! checkpoint 状态恢复与导入。

use super::*;

impl Scheduler {
    /// 合并导入 checkpoint 状态（不清空现有队列，供多 Spider 共享调度器使用）。
    pub async fn import_state(&self, pending: Vec<CrawlRequest>, seen: HashSet<String>) {
        for url in &seen {
            match self.strategy {
                DedupStrategy::Exact => {
                    self.seen_exact.insert(url.clone());
                }
                DedupStrategy::Fingerprint => {
                    if let Ok(h) = url.parse::<u64>() {
                        self.seen_fp.insert(h);
                    }
                }
            }
        }
        let mut g = self.heap.lock().await;
        for req in pending {
            match self.strategy {
                DedupStrategy::Exact => {
                    self.seen_exact.insert(req.url.clone());
                }
                DedupStrategy::Fingerprint => {
                    self.seen_fp.insert(fingerprint(&req.url));
                }
            }
            let seq = g.seq;
            g.heap.push(PrioritizedRequest { req, seq });
            g.seq += 1;
        }
        self.pending
            .store(g.heap.len() as usize, AtomicOrdering::Relaxed);
    }
}
