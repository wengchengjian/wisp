# Task 8 Review Package

## Commits
e6e99e7 feat: lib.rs 声明 cookie 模块并 re-export 公开 API

## Diff Stat
 src/lib.rs | 36 ++++++++++++++++++++++++++++++++++++
 1 file changed, 36 insertions(+)

## Full Diff
diff --git a/src/lib.rs b/src/lib.rs
index a0cc4ad..e587431 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -115,20 +115,23 @@ pub mod storage;
 /// 文本和属性处理工具。
 pub mod text;
 /// 内部辅助工具（URL 解析、随机后缀）。
 pub mod utils;
 
 // === 统一入口 ===
 pub use fetcher::{FetchClient, FetchClientConfig, FetchMode, Fetcher, FetcherBuilder};
 pub use fetcher::{Method, Request, Response};
 pub use stealth::TurnstileConfig;
 
+// === Cookie 管理 ===
+pub use cookie::{BrowserCookieJar, CfCookieJar, CfSession, Cookie, CookieJar, HttpCookieJar};
+
 // === 核心类型 ===
 pub use browser::{Browser, Page};
 pub use config::{LaunchOptions, ProxyConfig};
 pub use error::{
     BrowserError, McpError, NetworkError, ParseError, Result, StorageError, WispError,
 };
 
 pub use parser::{Node, NodeList};
 pub use proxy::RotationStrategy;
 pub use storage::{CachedResponse, ElementSnapshotRow, FileStore, MemoryStore, Store};
@@ -143,10 +146,43 @@ pub use storage::{
 pub use storage::SqliteStore;
 
 // === 爬虫引擎 ===
 pub use crawl::{
     ClosureSpider, CrawlEvent, CrawlStream, Engine, Items, JsonlWriter, Spider, SpiderBuilder,
 };
 pub use http::UaRotator;
 
 // === 底层类型（FetchClientConfig 公共字段需要） ===
 pub use http::DomainBlocker;
+
+#[cfg(test)]
+mod cookie_module_tests {
+    use std::sync::Arc;
+
+    /// 验证 cookie 模块的所有公开 API 可访问。
+    #[test]
+    fn cookie_module_public_api_accessible() {
+        use crate::cookie::{
+            BrowserCookieJar, CfCookieJar, CfSession, Cookie, CookieJar, HttpCookieJar,
+            MockCookieJar,
+        };
+
+        // 编译期检查：所有类型可命名
+        fn _check_cookie(c: Cookie) -> Cookie {
+            c
+        }
+        fn _check_session(s: CfSession) -> CfSession {
+            s
+        }
+        // trait object 可构造
+        let _: Arc<dyn CookieJar> = Arc::new(MockCookieJar::new());
+        // 各实现类型可命名（编译期检查）
+        fn _assert_implementations()
+        where
+            MockCookieJar: CookieJar,
+            HttpCookieJar: CookieJar,
+            BrowserCookieJar: CookieJar,
+            CfCookieJar: CookieJar,
+        {
+        }
+    }
+}
