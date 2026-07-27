# Review package: 05a921e..661739a

## Commits
661739a test: PR4 集成验证 Engine 配置聚合完整生命周期

## Files changed
 src/crawl/runner.rs | 46 ++++++++++++++++++++++++++++++++++++++++++++++
 1 file changed, 46 insertions(+)

## Diff
diff --git a/src/crawl/runner.rs b/src/crawl/runner.rs
index 598838d..cb8f638 100644
--- a/src/crawl/runner.rs
+++ b/src/crawl/runner.rs
@@ -745,11 +745,57 @@ mod tests {
 
         assert_eq!(
             flag.load(Ordering::SeqCst),
             0,
             "主循环不应等待 spawn 的任务"
         );
 
         tokio::time::sleep(Duration::from_millis(80)).await;
         assert_eq!(flag.load(Ordering::SeqCst), 1, "spawn 任务应完成");
     }
+
+    /// PR4 Task 8：集成验证 — Engine 完整生命周期使用 config() 路径访问配置。
+    #[tokio::test]
+    async fn test_engine_full_lifecycle_with_config_accessor() {
+        use futures::StreamExt;
+        use std::time::Duration;
+
+        struct OkSpider;
+        #[async_trait::async_trait]
+        impl super::super::Spider for OkSpider {
+            fn name(&self) -> &'static str { "ok" }
+            fn start_urls(&self) -> Vec<String> { vec![] }
+            async fn handle(&self, _resp: super::super::Response) -> (Vec<serde_json::Value>, Vec<super::super::Request>) {
+                (vec![], vec![])
+            }
+        }
+
+        let engine = super::Engine::infra()
+            .max_concurrent(2)
+            .max_pages(1)
+            .max_errors(10)
+            .fetch_mode(crate::fetcher::FetchMode::Http)
+            .obey_robots(false)
+            .download_delay_ms(10)
+            .build()
+            .unwrap();
+
+        // 验证所有 config 字段通过 config() 访问
+        assert_eq!(engine.config().max_concurrent, 2);
+        assert_eq!(engine.config().max_pages, 1);
+        assert_eq!(engine.config().max_errors, 10);
+        assert_eq!(engine.config().fetch_mode, crate::fetcher::FetchMode::Http);
+        assert!(!engine.config().obey_robots);
+        assert_eq!(engine.config().download_delay, Duration::from_millis(10));
+
+        // 验证 run_stream 不 panic
+        let mut stream = engine.run_stream(OkSpider).events();
+        let mut got_done = false;
+        while let Some(event) = stream.next().await {
+            if matches!(event, super::super::CrawlEvent::Done(_)) {
+                got_done = true;
+                break;
+            }
+        }
+        assert!(got_done, "应收到 Done 事件");
+    }
 }
