//! 运行守卫：防止同一 Engine 并发 run/run_stream。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wisp_core::error::Result;

/// 防止同一 Engine 实例被并发 run/run_stream 复用的运行守卫。
pub(crate) struct RunGuard(Arc<AtomicBool>);

impl RunGuard {
    pub(super) fn acquire(running: &Arc<AtomicBool>) -> Result<Self> {
        if running.swap(true, Ordering::SeqCst) {
            return Err(wisp_core::error::WispError::Engine(
                "Engine is already running. Concurrent run/run_stream on the same Engine is not supported. \
                 Create separate Engine instances for concurrent spiders."
                    .into(),
            ));
        }
        Ok(Self(running.clone()))
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
