# Review package: 70f2644..05a921e

## Commits
05a921e refactor: re-export EngineConfig 到 wisp::crawl

## Files changed
 src/crawl/mod.rs | 10 +++++++++-
 1 file changed, 9 insertions(+), 1 deletion(-)

## Diff
diff --git a/src/crawl/mod.rs b/src/crawl/mod.rs
index 812e780..679ec63 100644
--- a/src/crawl/mod.rs
+++ b/src/crawl/mod.rs
@@ -20,21 +20,21 @@ pub use runtime::items;
 pub use runtime::output;
 pub use runtime::robots;
 pub use scheduling::scheduler;
 pub use scheduling::stop;
 
 pub use adaptive::AdaptiveTracker;
 pub use auto::ModeRuleEngine;
 pub use builder::{ClosureSpider, SpiderBuilder};
 pub use engine::{fetch_page, fetch_page_inner, record_status};
 pub use items::{Items, JsonlWriter};
-pub use runner::{Engine, EngineBuilder};
+pub use runner::{Engine, EngineBuilder, EngineConfig};
 pub use state::CrawlState;
 pub use stop::{
     FnStopCondition, MaxErrors, MaxItems, MaxPages, NeverStop, StopCondition, StopContext, Timeout,
 };
 
 use async_trait::async_trait;
 use serde_json::Value;
 use std::collections::{HashMap, HashSet};
 use std::sync::Arc;
 use std::time::Duration;
@@ -475,11 +475,19 @@ mod tests {
         assert_eq!(nodes.iter().count(), 1);
     }
 
     #[test]
     fn test_method_as_str_returns_standard_verbs() {
         assert_eq!(Method::Get.as_str(), "GET");
         assert_eq!(Method::Post.as_str(), "POST");
         assert_eq!(Method::Put.as_str(), "PUT");
         assert_eq!(Method::Delete.as_str(), "DELETE");
     }
+
+    /// PR4 Task 7：验证 wisp::crawl::EngineConfig 公开可访问。
+    #[test]
+    fn test_engine_config_public_accessible() {
+        let config = crate::crawl::EngineConfig::default();
+        assert_eq!(config.max_concurrent, 8);
+        assert_eq!(config.max_pages, 1000);
+    }
 }
