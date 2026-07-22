## Commits
8e9e7a9 feat: Spider trait 加 patterns/matches/until 钩子

## Diff

diff --git a/src/crawl/mod.rs b/src/crawl/mod.rs
index d01656e..a6d851a 100644
--- a/src/crawl/mod.rs
+++ b/src/crawl/mod.rs
@@ -186,20 +186,42 @@ pub trait Spider: Send + Sync + 'static {
     /// 最大爬取深度。默认无限制。
     fn max_depth(&self) -> u32 { u32::MAX }
     /// 每次请求随机轮换 User-Agent。
     fn rotate_ua(&self) -> bool { false }
     /// 每个请求执行前的异步钩子。默认返回 Proceed。
     async fn on_before_request(&self, _req: &SpiderRequest) -> RequestAction {
         RequestAction::Proceed
     }
     /// Cron 表达式（标准 5 字段）。返回 None 表示立即执行一次（默认行为）。
     fn schedule(&self) -> Option<&str> { None }
+
+    // === 路由与终止（新增） ===
+
+    /// URL 匹配模式（字符串数组，内部自动编译为正则）。默认空 Vec（匹配所有）。
+    fn patterns(&self) -> Vec<String> { Vec::new() }
+
+    /// URL 匹配判定。默认实现遍历 patterns()，任一正则匹配即返回 true。
+    /// patterns() 为空时匹配所有 URL。
+    fn matches(&self, url: &str) -> bool {
+        let patterns = self.patterns();
+        if patterns.is_empty() {
+            return true;
+        }
+        patterns.iter().any(|p| {
+            regex::Regex::new(p).map(|re| re.is_match(url)).unwrap_or(false)
+        })
+    }
+
+    /// 终止条件。默认永不停止（由引擎 max_pages 兜底）。
+    fn until(&self) -> Arc<dyn StopCondition> {
+        Arc::new(NeverStop)
+    }
 }
 
 /// 默认阻塞状态码：401/403/407/429/444/500/502/503/504
 pub const BLOCKED_STATUS_CODES: &[u16] = &[401, 403, 407, 429, 444, 500, 502, 503, 504];
 
 /// Crawling statistics.
 #[derive(Debug, Clone, Default)]
 pub struct CrawlStats {
     pub items_scraped: usize,
     pub pages_crawled: usize,
