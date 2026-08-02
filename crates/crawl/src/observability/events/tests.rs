use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn test_event_bus_no_listeners() {
    let bus = EventBus::new();
    assert!(!bus.has_listeners());
    // emit should be no-op
    bus.emit(EngineEvent::CrawlStarted {
        spider: "test".into(),
        start_urls: 1,
    })
    .await;
}

#[tokio::test]
async fn test_event_bus_with_listener() {
    let mut bus = EventBus::new();
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = Arc::clone(&counter);

    bus.on(Arc::new(move |_event: EngineEvent| {
        let c = Arc::clone(&counter_clone);
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
        })
    }));

    assert!(bus.has_listeners());
    assert_eq!(bus.listener_count(), 1);

    bus.emit(EngineEvent::CrawlStarted {
        spider: "test".into(),
        start_urls: 1,
    })
    .await;
    bus.emit(EngineEvent::ItemScraped {
        url: "http://x.com".into(),
    })
    .await;

    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_metrics_listener() {
    let metrics = Arc::new(Metrics::new());
    let mut bus = EventBus::new();
    bus.on(metrics_listener(Arc::clone(&metrics)));

    bus.emit(EngineEvent::ResponseReceived {
        url: "http://x.com".into(),
        status: 200,
        elapsed_ms: 150,
        from_cache: false,
    })
    .await;

    bus.emit(EngineEvent::ItemScraped {
        url: "http://x.com".into(),
    })
    .await;

    assert_eq!(metrics.responses.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.items.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.avg_response_ms(), 150);
}
