//! Benchmark 专用：聚合 tracing span 的 wall clock duration，打印各阶段耗时百分比。
//!
//! 用 on_new_span（创建时记时间）而非 on_enter，因为 async span 可能多次
//! enter/exit（每次 poll），但创建到关闭的 wall clock = 该阶段真实耗时（含 I/O 等待）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::span::Id;
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

#[derive(Clone)]
pub struct TimingLayer {
    inner: Arc<Inner>,
}

struct Inner {
    /// span_id → 创建时间
    create_times: Mutex<HashMap<Id, Instant>>,
    /// span name → (总耗时, 调用次数)
    stats: Mutex<HashMap<String, (Duration, usize)>>,
}

impl TimingLayer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                create_times: Mutex::new(HashMap::new()),
                stats: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// 清空统计（每个 benchmark 级别前重置）
    pub fn reset(&self) {
        self.inner.create_times.lock().unwrap().clear();
        self.inner.stats.lock().unwrap().clear();
    }

    /// 按 total duration 降序打印各阶段耗时 + 百分比 + 调用次数
    pub fn print_summary(&self) {
        let stats = self.inner.stats.lock().unwrap();
        if stats.is_empty() {
            println!("  (no span data — subscriber not registered?)");
            return;
        }
        let mut entries: Vec<_> = stats.iter().collect();
        entries.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        let total = entries
            .iter()
            .find(|(name, _)| **name == "process_request")
            .map(|(_, (dur, _))| *dur)
            .unwrap_or_else(|| entries.iter().map(|(_, (d, _))| *d).max().unwrap_or_default());
        for (name, (dur, count)) in entries {
            let pct = if total.as_nanos() > 0 {
                dur.as_secs_f64() / total.as_secs_f64() * 100.0
            } else {
                0.0
            };
            println!(
                "  {:30} {:10.3} ms ({:5.1}%)  {} calls",
                name,
                dur.as_secs_f64() * 1000.0,
                pct,
                count
            );
        }
    }
}

impl<S> Layer<S> for TimingLayer
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        _attrs: &tracing::span::Attributes<'_>,
        id: &Id,
        _ctx: Context<'_, S>,
    ) {
        self.inner
            .create_times
            .lock()
            .unwrap()
            .insert(id.clone(), Instant::now());
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let created = self.inner.create_times.lock().unwrap().remove(&id);
        if let Some(created) = created {
            let dur = created.elapsed();
            let name = ctx
                .span(&id)
                .map(|s| s.name().to_string())
                .unwrap_or_default();
            let mut stats = self.inner.stats.lock().unwrap();
            let entry = stats.entry(name).or_insert((Duration::ZERO, 0));
            entry.0 += dur;
            entry.1 += 1;
        }
    }
}
