## Commits
371ee65 feat: SpiderBuilder/ClosureSpider 支持 patterns 与 until

## Diff

diff --git a/src/crawl/builder.rs b/src/crawl/builder.rs
index 9118ac8..221be7c 100644
--- a/src/crawl/builder.rs
+++ b/src/crawl/builder.rs
@@ -15,20 +15,21 @@
 //!         let doc = resp.parse().unwrap();
 //!         let items = doc.select(".quote").iter().map(|q| {
 //!             serde_json::json!({ "text": q.select_one(".text").map(|n| n.text()) })
 //!         }).collect();
 //!         (items, vec![])
 //!     })
 //!     .build();
 //! ```
 
 use std::collections::HashSet;
+use std::sync::Arc;
 use std::time::Duration;
 use async_trait::async_trait;
 use serde_json::Value;
 
 use super::{Spider, SpiderRequest, SpiderResponse};
 use crate::http;
 
 /// 瑙ｆ瀽闂寘绫诲瀷锛氭帴鏀?SpiderResponse锛岃繑鍥?(items, follow_requests)銆?
 pub type ParseFn = Box<dyn Fn(SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) + Send + Sync + 'static>;
 
@@ -46,40 +47,44 @@ pub struct SpiderBuilder {
     delay: Duration,
     obey_robots: bool,
     max_retries: u32,
     fetcher_config: http::Config,
     fetch_mode: crate::fetcher::FetchMode,
     auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
     auto_exclude: HashSet<String>,
     parse_fn: Option<ParseFn>,
     async_parse_fn: Option<AsyncParseFn>,
     is_blocked_fn: Option<Box<dyn Fn(&SpiderResponse) -> bool + Send + Sync + 'static>>,
+    patterns: Vec<String>,
+    until_cond: Arc<dyn super::stop::StopCondition>,
 }
 
 impl SpiderBuilder {
     /// 鍒涘缓鏂?SpiderBuilder锛坣ame 涓哄繀濉級銆?
     pub fn new(name: &str) -> Self {
         Self {
             name: name.to_string(),
             start_urls: Vec::new(),
             allowed_domains: HashSet::new(),
             concurrent: 8,
             delay: Duration::ZERO,
             obey_robots: true,
             max_retries: 3,
             fetcher_config: http::Config::default(),
             fetch_mode: crate::fetcher::FetchMode::Http,
             auto_rules: Vec::new(),
             auto_exclude: HashSet::new(),
             parse_fn: None,
             async_parse_fn: None,
             is_blocked_fn: None,
+            patterns: Vec::new(),
+            until_cond: Arc::new(super::NeverStop),
         }
     }
 
     /// 璁剧疆璧峰 URL 鍒楄〃銆?
     pub fn start_urls(mut self, urls: Vec<impl Into<String>>) -> Self {
         self.start_urls = urls.into_iter().map(|u| u.into()).collect();
         self
     }
 
     /// 璁剧疆鍏佽鐨勫煙鍚嶉泦鍚堛€?
@@ -167,20 +172,32 @@ impl SpiderBuilder {
 
     /// 鑷畾涔夐樆濉炴娴嬮€昏緫銆?
     pub fn is_blocked<F>(mut self, f: F) -> Self
     where
         F: Fn(&SpiderResponse) -> bool + Send + Sync + 'static,
     {
         self.is_blocked_fn = Some(Box::new(f));
         self
     }
 
+    /// 设置 URL 匹配模式（正则字符串数组）。任一匹配即处理该 URL。
+    pub fn patterns(mut self, patterns: Vec<String>) -> Self {
+        self.patterns = patterns;
+        self
+    }
+
+    /// 设置终止条件策略。
+    pub fn until<C: super::stop::StopCondition + 'static>(mut self, cond: C) -> Self {
+        self.until_cond = Arc::new(cond);
+        self
+    }
+
     /// 鏋勫缓 ClosureSpider 瀹炰緥銆?
     ///
     /// # Panics
     /// 鑻ユ湭璁剧疆 parse 鎴?parse_async 闂寘鍒?panic銆?
     pub fn build(self) -> ClosureSpider {
         assert!(
             self.parse_fn.is_some() || self.async_parse_fn.is_some(),
             "SpiderBuilder: 蹇呴』璁剧疆 parse() 鎴?parse_async() 闂寘"
         );
         ClosureSpider {
@@ -191,40 +208,44 @@ impl SpiderBuilder {
             delay: self.delay,
             obey_robots: self.obey_robots,
             max_retries: self.max_retries,
             fetcher_config: self.fetcher_config,
             fetch_mode: self.fetch_mode,
             auto_rules: self.auto_rules,
             auto_exclude: self.auto_exclude,
             parse_fn: self.parse_fn,
             async_parse_fn: self.async_parse_fn,
             is_blocked_fn: self.is_blocked_fn,
+            patterns: self.patterns,
+            until_cond: self.until_cond,
         }
     }
 }
 
 /// 鐢?SpiderBuilder 鏋勫缓鐨勯棴鍖呭紡 Spider銆?
 pub struct ClosureSpider {
     name: String,
     start_urls: Vec<String>,
     allowed_domains: HashSet<String>,
     concurrent: u32,
     delay: Duration,
     obey_robots: bool,
     max_retries: u32,
     fetcher_config: http::Config,
     fetch_mode: crate::fetcher::FetchMode,
     auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
     auto_exclude: HashSet<String>,
     parse_fn: Option<ParseFn>,
     async_parse_fn: Option<AsyncParseFn>,
     is_blocked_fn: Option<Box<dyn Fn(&SpiderResponse) -> bool + Send + Sync + 'static>>,
+    patterns: Vec<String>,
+    until_cond: Arc<dyn super::stop::StopCondition>,
 }
 
 #[async_trait]
 impl Spider for ClosureSpider {
     fn name(&self) -> &str { &self.name }
     fn start_urls(&self) -> Vec<String> { self.start_urls.clone() }
     fn allowed_domains(&self) -> HashSet<String> { self.allowed_domains.clone() }
     fn concurrent_requests(&self) -> u32 { self.concurrent }
     fn download_delay(&self) -> Duration { self.delay }
     fn obey_robots(&self) -> bool { self.obey_robots }
@@ -244,20 +265,26 @@ impl Spider for ClosureSpider {
         }
     }
 
     fn is_blocked(&self, resp: &SpiderResponse) -> bool {
         if let Some(ref f) = self.is_blocked_fn {
             f(resp)
         } else {
             super::BLOCKED_STATUS_CODES.contains(&resp.status)
         }
     }
+
+    fn patterns(&self) -> Vec<String> { self.patterns.clone() }
+
+    fn until(&self) -> Arc<dyn super::stop::StopCondition> {
+        Arc::clone(&self.until_cond)
+    }
 }
 
 #[cfg(test)]
 mod tests {
     use super::*;
     use serde_json::json;
 
     #[test]
     fn test_spider_builder_basic() {
         let spider = SpiderBuilder::new("test")
