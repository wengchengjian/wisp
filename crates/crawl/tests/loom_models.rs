#![cfg(feature = "loom")]
//! Loom model tests for scheduler/control/autoscale invariants.
//!
//! Production code currently depends on DashMap/Tokio sync primitives that
//! loom cannot model directly, so these tests model the same state transitions
//! with loom types.

use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use loom::sync::{Arc, Mutex, Notify, RwLock};
use loom::thread;
use std::collections::{BinaryHeap, HashSet};

#[test]
fn scheduler_pending_counter_matches_queue_len() {
    loom::model(|| {
        let queue = Arc::new(Mutex::new(BinaryHeap::<u32>::new()));
        let pending = Arc::new(AtomicUsize::new(0));

        let q1 = Arc::clone(&queue);
        let p1 = Arc::clone(&pending);
        let t1 = thread::spawn(move || {
            for _ in 0..3 {
                q1.lock().unwrap().push(1);
                p1.fetch_add(1, Ordering::Relaxed);
            }
        });

        let q2 = Arc::clone(&queue);
        let p2 = Arc::clone(&pending);
        let t2 = thread::spawn(move || {
            for _ in 0..2 {
                q2.lock().unwrap().push(2);
                p2.fetch_add(1, Ordering::Relaxed);
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();

        assert_eq!(queue.lock().unwrap().len(), pending.load(Ordering::Relaxed));
    });
}

#[test]
fn control_shutdown_wakes_waiting_worker() {
    loom::model(|| {
        let paused = Arc::new(RwLock::new(HashSet::<String>::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());

        let waiter_paused = Arc::clone(&paused);
        let waiter_shutdown = Arc::clone(&shutdown);
        let waiter_notify = Arc::clone(&notify);
        let waiter = thread::spawn(move || {
            loop {
                if waiter_shutdown.load(Ordering::SeqCst) {
                    return;
                }
                if !waiter_paused.read().unwrap().contains("a") {
                    return;
                }
                waiter_notify.wait();
            }
        });

        paused.write().unwrap().insert("a".to_string());
        shutdown.store(true, Ordering::SeqCst);
        notify.notify();

        waiter.join().unwrap();
    });
}

#[test]
fn autoscale_notify_observes_updated_concurrency() {
    loom::model(|| {
        let current = Arc::new(AtomicUsize::new(2));
        let lock = Arc::new(Mutex::new(()));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let current = Arc::clone(&current);
            let lock = Arc::clone(&lock);
            handles.push(thread::spawn(move || {
                let _guard = lock.lock().unwrap();
                let value = current.load(Ordering::SeqCst);
                current.store(value + 1, Ordering::SeqCst);
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(current.load(Ordering::SeqCst), 4);
    });
}
