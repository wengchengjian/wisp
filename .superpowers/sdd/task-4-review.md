## Commits
1b31bcc fix: SpiderResponse 加 from_cache，修复缓存命中误统计 pages_crawled

## Stat

 src/crawl/builder.rs      | 3 +++
 src/crawl/engine.rs       | 8 +++++++-
 src/crawl/mod.rs          | 4 ++++
 tests/builder_api_test.rs | 4 ++++
 tests/real_scrape_test.rs | 2 ++
 5 files changed, 20 insertions(+), 1 deletion(-)

## Diff

diff --git a/src/crawl/builder.rs b/src/crawl/builder.rs
index 7d6118b..9118ac8 100644
--- a/src/crawl/builder.rs
+++ b/src/crawl/builder.rs
@@ -309,20 +309,21 @@ mod tests {
             })
             .build();
 
         let resp = SpiderResponse {
             url: "https://example.com/".into(),
             status: 200,
             headers: Default::default(),
             body: b"<html><body><h1>Hello</h1></body></html>".to_vec(),
             request: SpiderRequest::get("https://example.com/"),
             tracker: None,
+            from_cache: false,
         };
 
         let (items, follows) = spider.parse(resp).await;
         assert_eq!(items.len(), 1);
         assert_eq!(items[0]["title"], "Hello");
         assert!(follows.is_empty());
     }
 
     #[tokio::test]
     async fn test_closure_spider_parse_async() {
@@ -335,20 +336,21 @@ mod tests {
             })
             .build();
 
         let resp = SpiderResponse {
             url: "https://example.com/".into(),
             status: 200,
             headers: Default::default(),
             body: b"<html><body><p>World</p></body></html>".to_vec(),
             request: SpiderRequest::get("https://example.com/"),
             tracker: None,
+            from_cache: false,
         };
 
         let (items, _) = spider.parse(resp).await;
         assert_eq!(items[0]["text"], "World");
     }
 
     #[test]
     fn test_closure_spider_custom_is_blocked() {
         let spider = SpiderBuilder::new("test")
             .start_urls(Vec::<String>::new())
@@ -356,20 +358,21 @@ mod tests {
             .is_blocked(|resp| resp.body.windows(7).any(|w| w == b"blocked"))
             .build();
 
         let resp = SpiderResponse {
             url: "http://x.com".into(),
             status: 200,
             headers: Default::default(),
             body: b"you are blocked".to_vec(),
             request: SpiderRequest::get("http://x.com"),
             tracker: None,
+            from_cache: false,
         };
         assert!(spider.is_blocked(&resp));
 
         let ok_resp = SpiderResponse {
             body: b"welcome".to_vec(),
             ..resp
         };
         assert!(!spider.is_blocked(&ok_resp));
     }
 }
diff --git a/src/crawl/engine.rs b/src/crawl/engine.rs
index 9e316f1..68933f9 100644
--- a/src/crawl/engine.rs
+++ b/src/crawl/engine.rs
@@ -100,20 +100,21 @@ pub(crate) async fn process_request(ctx: &EngineContext, req: SpiderRequest) {
     // 2. 内存缓存检查 (RequestCache)
     if let Some(ref rc) = ctx.request_cache {
         if let Some(entry) = rc.get(&req.url).await {
             let resp = SpiderResponse {
                 url: req.url.clone(),
                 status: entry.status,
                 headers: entry.headers,
                 body: entry.body,
                 request: req.clone(),
                 tracker: None,
+                from_cache: true,
             };
             ctx.stats_cache_hits.fetch_add(1, Ordering::SeqCst);
             record_status(ctx, resp.status).await;
             // 直接跳到处理结果阶段
             return process_response(ctx, resp, &req).await;
         }
     }
 
     // 3. 开发模式 SQLite 缓存检查
     let method_str = match req.method {
@@ -135,20 +136,21 @@ pub(crate) async fn process_request(ctx: &EngineContext, req: SpiderRequest) {
 
     if let Some(cached) = cached_resp {
         // 命中缓存
         let resp = SpiderResponse {
             url: req.url.clone(),
             status: cached.status,
             headers: cached.headers,
             body: cached.body,
             request: req.clone(),
             tracker: None,
+            from_cache: true,
         };
         ctx.stats_cache_hits.fetch_add(1, Ordering::SeqCst);
         record_status(ctx, resp.status).await;
         final_resp = Some(resp);
     } else {
         // 3. Robots 检查
         if ctx.obey_robots {
             let allowed_flag = {
                 let mut rc = ctx.robots_cache.lock().await;
                 rc.is_allowed(&ctx.client, &req.url).await
@@ -209,21 +211,23 @@ pub(crate) async fn process_request(ctx: &EngineContext, req: SpiderRequest) {
         process_response(ctx, resp, &req).await;
     } else if let Some(err) = last_error {
         if let Some(ref tx) = ctx.tx {
             let _ = tx.send(CrawlEvent::Error { url: req.url.clone(), error: err }).await;
         }
     }
 }
 
 /// 处理已获取的响应：parse → Auto 升级 → items → events。
 pub(crate) async fn process_response(ctx: &EngineContext, resp: SpiderResponse, req: &SpiderRequest) {
-    ctx.stats_pages.fetch_add(1, Ordering::SeqCst);
+    if !resp.from_cache {
+        ctx.stats_pages.fetch_add(1, Ordering::SeqCst);
+    }
     let page_url = resp.url.clone();
 
     let tracker_ref = resp.tracker.clone();
     let (mut items, mut follows) = ctx.spider.parse(resp).await;
 
     // Auto 升级检查
     if ctx.fetch_mode == FetchMode::Auto {
         if let Some(result) = auto_upgrade_check(ctx, &tracker_ref, &page_url, req).await {
             items = result.0;
             follows = result.1;
@@ -440,20 +444,21 @@ pub(crate) async fn fetch_page_inner(
             builder = builder.proxy(proxy);
         }
         let resp = builder.get(&req.url).await?;
         return Ok(SpiderResponse {
             url: resp.url.clone(),
             status: resp.status,
             headers: resp.headers.clone(),
             body: resp.body.clone(),
             request: req.clone(),
             tracker: None,
+            from_cache: false,
         });
     }
 
     // Http 模式
     let effective_client: Client;
     let need_custom_client = proxy_url.is_some() || config.rotate_ua;
     let use_client = if need_custom_client {
         let mut builder = Client::builder()
             .timeout(client.config_ref().timeout);
         if let Some(proxy) = proxy_url {
@@ -476,20 +481,21 @@ pub(crate) async fn fetch_page_inner(
         Method::Delete => use_client.delete(&req.url).await?,
     };
 
     Ok(SpiderResponse {
         url: resp.url.clone(),
         status: resp.status,
         headers: resp.headers.clone(),
         body: resp.body.clone(),
         request: req.clone(),
         tracker: None,
+        from_cache: false,
     })
 }
 
 // === InFlightGuard ===
 
 pub(crate) struct InFlightGuard {
     pub counter: Arc<AtomicUsize>,
 }
 
 impl Drop for InFlightGuard {
diff --git a/src/crawl/mod.rs b/src/crawl/mod.rs
index 3bd798a..d01656e 100644
--- a/src/crawl/mod.rs
+++ b/src/crawl/mod.rs
@@ -89,20 +89,23 @@ impl SpiderRequest {
 #[derive(Debug, Clone)]
 pub struct SpiderResponse {
     pub url: String,
     pub status: u16,
     pub headers: HashMap<String, String>,
     pub body: Vec<u8>,
     pub request: SpiderRequest,
     /// Auto 模式选择器追踪器
     #[doc(hidden)]
     pub tracker: Option<Arc<std::sync::Mutex<auto::SelectorTracker>>>,
+    /// 是否来自缓存（缓存命中不算 pages_crawled）。
+    #[doc(hidden)]
+    pub from_cache: bool,
 }
 
 impl SpiderResponse {
     pub fn text(&self) -> Result<String> {
         String::from_utf8(self.body.clone())
             .map_err(|e| WispError::CdpError(format!("utf8 decode: {e}")))
     }
     pub fn parse(&self) -> Result<Node> {
         let text = self.text()?;
         Ok(Node::from_html(&text))
@@ -633,20 +636,21 @@ mod tests {
             async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) { (vec![], vec![]) }
         }
         let spider = DummySpider;
         let blocked_resp = SpiderResponse {
             url: "http://example.com".into(),
             status: 403,
             headers: HashMap::new(),
             body: vec![],
             request: SpiderRequest::get("http://example.com"),
             tracker: None,
+            from_cache: false,
         };
         assert!(spider.is_blocked(&blocked_resp));
         let ok_resp = SpiderResponse { status: 200, ..blocked_resp };
         assert!(!spider.is_blocked(&ok_resp));
     }
 
     async fn spawn_html_server(html: &'static str) -> String {
         use tokio::io::{AsyncReadExt, AsyncWriteExt};
         use tokio::net::TcpListener;
         let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
diff --git a/tests/builder_api_test.rs b/tests/builder_api_test.rs
index 833e49f..8c40af8 100644
--- a/tests/builder_api_test.rs
+++ b/tests/builder_api_test.rs
@@ -64,67 +64,71 @@ async fn test_spider_builder_parse_with_follow() {
         })
         .build();
 
     let resp = SpiderResponse {
         url: "https://example.com/".into(),
         status: 200,
         headers: Default::default(),
         body: b"<html><body><h1>Home</h1></body></html>".to_vec(),
         request: SpiderRequest::get("https://example.com/"),
         tracker: None,
+        from_cache: false,
     };
 
     let (items, follows) = spider.parse(resp).await;
     assert_eq!(items.len(), 1);
     assert_eq!(items[0]["title"], "Home");
     assert_eq!(follows.len(), 1);
 }
 
 // === SpiderResponse.follow() tests ===
 
 #[test]
 fn test_response_follow_absolute_url() {
     let resp = SpiderResponse {
         url: "https://example.com/page1".into(),
         status: 200,
         headers: Default::default(),
         body: vec![],
         request: SpiderRequest::get("https://example.com/page1"),
         tracker: None,
+        from_cache: false,
     };
     let req = resp.follow("https://other.com/page2").unwrap();
     assert_eq!(req.url, "https://other.com/page2");
 }
 
 #[test]
 fn test_response_follow_relative_path() {
     let resp = SpiderResponse {
         url: "https://example.com/dir/page1".into(),
         status: 200,
         headers: Default::default(),
         body: vec![],
         request: SpiderRequest::get("https://example.com/dir/page1"),
         tracker: None,
+        from_cache: false,
     };
     let req = resp.follow("/page2").unwrap();
     assert_eq!(req.url, "https://example.com/page2");
 }
 
 #[test]
 fn test_response_follow_with_callback() {
     let resp = SpiderResponse {
         url: "https://example.com/".into(),
         status: 200,
         headers: Default::default(),
         body: vec![],
         request: SpiderRequest::get("https://example.com/"),
         tracker: None,
+        from_cache: false,
     };
     let req = resp.follow_with("/detail", "parse_detail").unwrap();
     assert_eq!(req.url, "https://example.com/detail");
     assert_eq!(req.callback, Some("parse_detail".to_string()));
 }
 
 // === Engine::builder() test ===
 
 #[tokio::test]
 async fn test_engine_builder_local_server() {
diff --git a/tests/real_scrape_test.rs b/tests/real_scrape_test.rs
index be5bc62..cdf94a0 100644
--- a/tests/real_scrape_test.rs
+++ b/tests/real_scrape_test.rs
@@ -227,20 +227,22 @@ async fn test_response_follow_pagination() {
     let fetch_resp = resp.unwrap();
     let doc = fetch_resp.parse().unwrap();
 
     // 鏋勯€?SpiderResponse 鏉ユ祴璇?follow()
     let spider_resp = SpiderResponse {
         url: "https://quotes.toscrape.com/".into(),
         status: 200,
         headers: Default::default(),
         body: fetch_resp.body.clone(),
         request: SpiderRequest::get("https://quotes.toscrape.com/"),
+        tracker: None,
+        from_cache: false,
     };
 
     // 鑾峰彇涓嬩竴椤甸摼鎺?
     let next_href = doc.select_one(".next a")
         .and_then(|a| a.attr("href"));
 
     if let Some(href) = next_href {
         let follow_req = spider_resp.follow(&href);
         assert!(follow_req.is_some(), "follow() 搴旇繑鍥?Some");
         let req = follow_req.unwrap();
