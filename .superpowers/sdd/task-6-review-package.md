diff --git a/src/crawl/observability/events.rs b/src/crawl/observability/events.rs
index 6d9aabb..8a30e22 100644
--- a/src/crawl/observability/events.rs
+++ b/src/crawl/observability/events.rs
@@ -99,7 +99,7 @@ pub enum EngineEvent {
 }
 
 /// 事件监听器签名。
-pub type EventListener = Arc<dyn Fn(EngineEvent) -> BoxFuture<'static, ()> + Send + Sync>;
+pub type EventListener = Arc<dyn Fn(Arc<EngineEvent>) -> BoxFuture<'static, ()> + Send + Sync>;
 
 /// 事件总线：管理监听器并分发事件。
 pub struct EventBus {
@@ -121,17 +121,18 @@ impl EventBus {
 
     /// 发射事件（无 listener 时为 no-op）。
     ///
-    /// OPTIMIZE: 旧实现 `for listener in &self.listeners { listener(event.clone()).await }`
-    /// 串行 await，N 个 listener 的总延迟 = sum(单 listener)。慢 listener 阻塞后续。
-    /// 改用 FuturesUnordered 并发 await，所有 listener 同时执行，总延迟 = max(单 listener)。
+    /// OPTIMIZE: 旧实现 `listener(event.clone())` 每次 emit N-1 次 EngineEvent 深拷贝。
+    /// 改用 `Arc<EngineEvent>` 共享：1 次 Arc 分配 + N-1 次 Arc::clone（原子计数器自增，无堆分配）。
+    /// 同时用 FuturesUnordered 并发 await，总延迟 = max(单 listener)。
     pub async fn emit(&self, event: EngineEvent) {
         if self.listeners.is_empty() {
             return;
         }
+        let event = Arc::new(event);
         let mut futures: FuturesUnordered<_> = self
             .listeners
             .iter()
-            .map(|listener| listener(event.clone()))
+            .map(|listener| listener(Arc::clone(&event)))
             .collect();
         while futures.next().await.is_some() {}
     }
@@ -155,9 +156,9 @@ impl Default for EventBus {
 
 /// 便捷构造：日志监听器（tracing 输出）。
 pub fn logging_listener() -> EventListener {
-    Arc::new(|event: EngineEvent| {
+    Arc::new(|event: Arc<EngineEvent>| {
         Box::pin(async move {
-            match &event {
+            match &*event {
                 EngineEvent::CrawlStarted { spider, start_urls } => {
                     tracing::info!("Crawl started: {} ({} URLs)", spider, start_urls);
                 }
@@ -185,10 +186,10 @@ pub fn logging_listener() -> EventListener {
 
 /// 便捷构造：指标收集监听器。
 pub fn metrics_listener(metrics: Arc<Metrics>) -> EventListener {
-    Arc::new(move |event: EngineEvent| {
+    Arc::new(move |event: Arc<EngineEvent>| {
         let metrics = Arc::clone(&metrics);
         Box::pin(async move {
-            match event {
+            match &*event {
                 EngineEvent::ResponseReceived {
                     elapsed_ms,
                     from_cache,
@@ -197,14 +198,14 @@ pub fn metrics_listener(metrics: Arc<Metrics>) -> EventListener {
                     metrics
                         .responses
                         .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
-                    if from_cache {
+                    if *from_cache {
                         metrics
                             .cache_hits
                             .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                     }
                     metrics
                         .total_elapsed_ms
-                        .fetch_add(elapsed_ms, std::sync::atomic::Ordering::Relaxed);
+                        .fetch_add(*elapsed_ms, std::sync::atomic::Ordering::Relaxed);
                 }
                 EngineEvent::ItemScraped { .. } => {
                     metrics
@@ -278,7 +279,7 @@ mod tests {
         let counter = Arc::new(AtomicUsize::new(0));
         let counter_clone = Arc::clone(&counter);
 
-        bus.on(Arc::new(move |_event: EngineEvent| {
+        bus.on(Arc::new(move |_event: Arc<EngineEvent>| {
             let c = Arc::clone(&counter_clone);
             Box::pin(async move {
                 c.fetch_add(1, Ordering::SeqCst);
@@ -346,7 +347,7 @@ mod tests {
         for label in ["l1", "l2", "l3"] {
             let c = Arc::clone(&counter);
             let o = Arc::clone(&order);
-            bus.on(Arc::new(move |_event: EngineEvent| {
+            bus.on(Arc::new(move |_event: Arc<EngineEvent>| {
                 let c = Arc::clone(&c);
                 let o = Arc::clone(&o);
                 Box::pin(async move {
feba14b perf(events): EventListener 改 Arc<EngineEvent> 共享事件无 clone
 src/crawl/observability/events.rs | 27 ++++++++++++++-------------
 1 file changed, 14 insertions(+), 13 deletions(-)
