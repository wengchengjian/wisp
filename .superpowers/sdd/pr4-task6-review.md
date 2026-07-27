# Review package: 8859255..70f2644

## Commits
70f2644 refactor: CrawlContext 增加 config 字段，中间件可访问完整配置

## Files changed
 src/crawl/engine.rs             |  1 +
 src/crawl/middleware/builtin.rs |  1 +
 src/crawl/middleware/mod.rs     | 37 +++++++++++++++++++++++++++++++++++++
 3 files changed, 39 insertions(+)

## Diff
diff --git a/src/crawl/engine.rs b/src/crawl/engine.rs
index e5f238e..8571923 100644
--- a/src/crawl/engine.rs
+++ b/src/crawl/engine.rs
@@ -520,20 +520,21 @@ pub(crate) fn emit_error_event(ctx: &EngineContext, url: &str, err: &str) {
 /// 从 EngineContext 构建中间件用的 CrawlContext 只读视图。
 pub(crate) fn build_crawl_context(ctx: &EngineContext) -> middleware::CrawlContext {
     middleware::CrawlContext {
         spider_name: ctx.state.spider.name().to_string(),
         fetch_mode: ctx.config.fetch_mode,
         max_concurrent: ctx.config.max_concurrent,
         max_pages: ctx.config.max_pages,
         obey_robots: ctx.config.obey_robots,
         pages_crawled: ctx.state.stats.pages.load(Ordering::SeqCst),
         errors: ctx.state.stats.errors.load(Ordering::SeqCst),
+        config: Arc::clone(&ctx.config),
     }
 }
 
 /// 同步记录状态码计数（DashMap entry 原子累加，无 await）。
 #[doc(hidden)]
 pub fn record_status(stats: &Arc<SpiderStats>, status: u16) {
     stats
         .status_codes
         .entry(status)
         .and_modify(|c| {
diff --git a/src/crawl/middleware/builtin.rs b/src/crawl/middleware/builtin.rs
index cb7ce63..b11c87b 100644
--- a/src/crawl/middleware/builtin.rs
+++ b/src/crawl/middleware/builtin.rs
@@ -807,20 +807,21 @@ mod tests {
 
     fn make_ctx() -> CrawlContext {
         CrawlContext {
             spider_name: "test".into(),
             fetch_mode: FetchMode::Http,
             max_concurrent: 8,
             max_pages: 1000,
             obey_robots: false,
             pages_crawled: 0,
             errors: 0,
+            config: std::sync::Arc::new(crate::crawl::runner::EngineConfig::default()),
         }
     }
 
     #[tokio::test]
     async fn test_ua_rotation_middleware() {
         let mw = UaRotationMiddleware::desktop();
         let ctx = make_ctx();
         let mut req = make_req();
         let action = mw.process_request(&mut req, &ctx).await;
         assert_eq!(action, MwAction::Modified);
diff --git a/src/crawl/middleware/mod.rs b/src/crawl/middleware/mod.rs
index bb8c99a..791b794 100644
--- a/src/crawl/middleware/mod.rs
+++ b/src/crawl/middleware/mod.rs
@@ -73,36 +73,47 @@ pub enum ErrorAction {
     Propagate,
     /// 重试此请求
     Retry,
     /// 忽略错误，跳过此请求
     Ignore,
 }
 
 /// 引擎上下文只读视图（暴露给中间件）。
 ///
 /// 中间件可读取引擎级配置和统计信息，用于决策（如根据已爬取页数调整策略）。
+/// PR4 重构：新增 config 字段（Arc<EngineConfig>），中间件可通过 config() 访问完整配置。
 #[derive(Debug, Clone)]
 pub struct CrawlContext {
     /// Spider 名称
     pub spider_name: String,
     /// 当前抓取模式
     pub fetch_mode: FetchMode,
     /// 最大并发数
     pub max_concurrent: usize,
     /// 最大爬取页数
     pub max_pages: usize,
     /// 是否遵守 robots.txt
     pub obey_robots: bool,
     /// 已爬取页数（只读快照）
     pub pages_crawled: usize,
     /// 错误数（只读快照）
     pub errors: usize,
+    /// 完整引擎配置（Arc 共享，中间件可访问任意配置字段）。
+    pub config: std::sync::Arc<crate::crawl::runner::EngineConfig>,
+}
+
+impl CrawlContext {
+    /// 获取完整引擎配置引用。
+    #[must_use]
+    pub fn config(&self) -> &crate::crawl::runner::EngineConfig {
+        &self.config
+    }
 }
 
 // === Middleware trait ===
 
 /// 请求/响应中间件（Downloader Middleware 等价）。
 ///
 /// 实现此 trait 以在请求发出前/响应返回后执行自定义逻辑。
 /// 所有方法都有默认实现，只需覆盖需要的方法。
 #[async_trait]
 pub trait Middleware: Send + Sync {
@@ -262,10 +273,36 @@ impl MiddlewareChain {
         }
     }
 
     /// 关闭所有 pipeline（爬取结束后调用）。
     pub(crate) async fn run_pipelines_close(&self, ctx: &CrawlContext) {
         for pipeline in &self.pipelines {
             pipeline.close(ctx).await;
         }
     }
 }
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    /// PR4 Task 6：验证 CrawlContext 持有 config 字段并提供 config() 方法。
+    #[test]
+    fn test_crawl_context_has_config_field() {
+        let config = std::sync::Arc::new(crate::crawl::runner::EngineConfig::default());
+        let ctx = CrawlContext {
+            spider_name: "test".to_string(),
+            fetch_mode: crate::fetcher::FetchMode::Http,
+            max_concurrent: 8,
+            max_pages: 100,
+            obey_robots: false,
+            pages_crawled: 0,
+            errors: 0,
+            config: std::sync::Arc::clone(&config),
+        };
+        // 验证 config() 方法返回 &EngineConfig
+        let cfg: &crate::crawl::runner::EngineConfig = ctx.config();
+        assert_eq!(cfg.max_concurrent, 8);
+        assert_eq!(cfg.max_pages, 1000); // default
+        assert_eq!(cfg.fetch_mode, crate::fetcher::FetchMode::Auto); // default
+    }
+}
