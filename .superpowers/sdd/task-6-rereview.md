# Task 6 Re-Review Package (after fix)

## Commits
09ac90b 修复 Task 6 审查问题：删除空测试并将 unwrap 改为 expect

## Diff Stat
 src/fetcher/client.rs | 11 +----------
 1 file changed, 1 insertion(+), 10 deletions(-)

## Full Diff
diff --git a/src/fetcher/client.rs b/src/fetcher/client.rs
index dd9eadd..6506e39 100644
--- a/src/fetcher/client.rs
+++ b/src/fetcher/client.rs
@@ -579,33 +579,24 @@ mod tests {
             name: "test".into(),
             value: "v".into(),
             domain: "example.com".into(),
             path: "/".into(),
             secure: false,
             http_only: false,
             same_site: None,
             expires: None,
         };
         jar.set(cookie).await;
-        let url = Url::parse("https://example.com/").unwrap();
+        let url = Url::parse("https://example.com/").expect("合法 URL");
         let cookies = jar.get(&url).await;
         assert_eq!(cookies.len(), 1);
         assert_eq!(cookies[0].name, "test");
     }
 
-    #[test]
-    fn fetch_client_no_longer_has_cf_cache_field() {
-        // 编译期验证：FetchClient 不再有 cf_cache 字段
-        // 如果 cf_cache 字段仍存在，下面的 type alias 会失败
-        fn _assert_no_cf_cache_field(_client: &FetchClient) {}
-        // 通过反射式检查：如果 has_cf_cookies 方法仍存在，编译会失败
-        // （下面的代码尝试调用不应存在的方法，编译失败说明未完成迁移）
-    }
-
     #[test]
     fn fetch_client_config_still_has_cf_fields() {
         // 验证 FetchClientConfig 仍保留 cf_cookie_ttl/cf_data_dir（供 StealthStrategy 在 PR2 使用）
         let config = FetchClientConfig::default();
         assert_eq!(config.cf_cookie_ttl, std::time::Duration::from_mins(30));
         assert_eq!(config.cf_data_dir, std::path::PathBuf::from("wisp-data"));
     }
 }
