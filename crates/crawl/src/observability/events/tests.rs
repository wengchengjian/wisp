use super::*;
use crate::CrawlEvent;
use crate::Item;
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn item(value: Value) -> Item {
    Item::new(value, "https://example.com", "test", None)
}

#[tokio::test]
async fn test_event_bus_no_listeners() {
    let bus = EventBus::new();
    assert!(!bus.has_listeners());
    // emit should be no-op
    bus.emit(CrawlEvent::CrawlStarted {
        spider: "test".into(),
        start_urls: 1,
    })
    .await;
}

#[tokio::test]
async fn test_event_bus_with_listener() {
    let bus = EventBus::new();
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = Arc::clone(&counter);

    bus.on(Arc::new(move |_event: CrawlEvent| {
        let c = Arc::clone(&counter_clone);
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
        })
    }));

    assert!(bus.has_listeners());
    assert_eq!(bus.listener_count(), 1);

    bus.emit(CrawlEvent::CrawlStarted {
        spider: "test".into(),
        start_urls: 1,
    })
    .await;
    bus.emit(CrawlEvent::Item(item(json!(1)))).await;

    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_event_bus_accepts_async_closure() {
    let bus = EventBus::new();
    bus.on(|_event: CrawlEvent| async move {});
    assert!(bus.has_listeners());
    bus.emit(CrawlEvent::CrawlStarted {
        spider: "test".into(),
        start_urls: 1,
    })
    .await;
}

#[tokio::test]
async fn test_subscription_fans_out_to_all_subscribers() {
    let bus = EventBus::new();
    let mut a = bus.subscribe(16);
    let mut b = bus.subscribe(16);

    bus.emit(CrawlEvent::Item(item(json!({ "n": 1 })))).await;

    assert!(matches!(a.next().await, Some(CrawlEvent::Item(_))));
    assert!(matches!(b.next().await, Some(CrawlEvent::Item(_))));
}

#[tokio::test]
async fn test_subscription_unsubscribes_on_drop() {
    let bus = EventBus::new();
    let sub = bus.subscribe(16);
    assert!(bus.listener_count() > 0);

    drop(sub);
    assert_eq!(bus.listener_count(), 0);
}

#[tokio::test]
async fn test_metrics_listener() {
    let metrics = Arc::new(Metrics::new());
    let bus = EventBus::new();
    bus.on(metrics_listener(Arc::clone(&metrics)));

    bus.emit(CrawlEvent::ResponseReceived {
        url: "http://x.com".into(),
        status: 200,
        elapsed_ms: 150,
        from_cache: false,
    })
    .await;

    bus.emit(CrawlEvent::Item(item(json!({ "x": 1 })))).await;

    assert_eq!(metrics.responses.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.items.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.avg_response_ms(), 150);
}
