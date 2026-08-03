# Engine events use one CrawlEvent seam

Status: accepted

CrawlEvent is the only event type in the crawl engine. Engine paths emit a
fact once; listeners registered with EventBus and stream subscribers created
by `subscribe()` all receive the same event. EngineEvent is removed.
ItemScraped merged into Item, ErrorOccurred split into Retry and Error, and
RequestScheduled plus ConcurrencyChanged were removed as unused telemetry.

EventBus keeps an awaited-listener core for reliable, ordered delivery and
adds a bounded mpsc `subscribe()` adapter with a Subscription guard that
unsubscribes on drop. Broadcast-style lossy channels were rejected because
metrics and retry semantics must not drop events.
