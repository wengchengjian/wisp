# Task 5 Review Package

## Commits
1afdd14 feat: StorageError 新增 NotFound/Serialization/Backend/Corrupted 变体

## Diff Stat
 src/error.rs       | 85 +++++++++++++++++++++++++++++++++++++++++++++++++++++-
 src/storage/mod.rs |  8 ++---
 2 files changed, 88 insertions(+), 5 deletions(-)

## Full Diff
diff --git a/src/error.rs b/src/error.rs
index bdc745b..da2d9a8 100644
--- a/src/error.rs
+++ b/src/error.rs
@@ -160,23 +160,48 @@ pub enum McpError {
     UnknownTool(String),
 }
 
 // ============================================================================
 // 存储错误
 // ============================================================================
 
 /// SQLite / 持久化存储相关错误。
 #[derive(Debug, Error)]
 pub enum StorageError {
-    /// 通用存储错误。
+    /// 通用存储错误（保留向后兼容，新代码应使用具体变体）。
     #[error("Storage error: {0}")]
     General(String),
+
+    /// 键不存在（namespace + key 定位）。
+    #[error("Key not found in namespace {namespace}: {key}")]
+    NotFound {
+        /// 命名空间（如 "checkpoint"/"element"/"response"）。
+        namespace: String,
+        /// 键名。
+        key: String,
+    },
+
+    /// 序列化/反序列化失败。
+    #[error("Serialization failed: {0}")]
+    Serialization(String),
+
+    /// 后端错误（SQLite/文件系统等底层错误）。
+    #[error("Backend error: {0}")]
+    Backend(String),
+
+    /// 数据损坏（存储的内容无法解析）。
+    #[error("Data corrupted: {0}")]
+    Corrupted(String),
+
+    /// IO 错误。
+    #[error("IO error: {0}")]
+    Io(#[from] std::io::Error),
 }
 
 // ============================================================================
 // 顶层统一错误
 // ============================================================================
 
 /// Wisp 统一错误类型。
 ///
 /// 按领域分类为子枚举，通过 `#[from]` 支持 `?` 自动转换：
 /// - `Browser(...)` — 浏览器 / CDP
@@ -218,10 +243,68 @@ pub enum WispError {
     #[error("Timeout: {0}")]
     Timeout(String),
 
     /// 系统 IO 错误
     #[error("IO error: {0}")]
     Io(#[from] std::io::Error),
 }
 
 /// Wisp 统一结果类型。
 pub type Result<T> = std::result::Result<T, WispError>;
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    #[test]
+    fn storage_error_general_display() {
+        let e = StorageError::General("msg".into());
+        assert_eq!(e.to_string(), "Storage error: msg");
+    }
+
+    #[test]
+    fn storage_error_not_found_display() {
+        let e = StorageError::NotFound {
+            namespace: "checkpoint".into(),
+            key: "spider1".into(),
+        };
+        assert_eq!(
+            e.to_string(),
+            "Key not found in namespace checkpoint: spider1"
+        );
+    }
+
+    #[test]
+    fn storage_error_serialization_display() {
+        let e = StorageError::Serialization("bad json".into());
+        assert_eq!(e.to_string(), "Serialization failed: bad json");
+    }
+
+    #[test]
+    fn storage_error_backend_display() {
+        let e = StorageError::Backend("sqlite locked".into());
+        assert_eq!(e.to_string(), "Backend error: sqlite locked");
+    }
+
+    #[test]
+    fn storage_error_corrupted_display() {
+        let e = StorageError::Corrupted("invalid magic".into());
+        assert_eq!(e.to_string(), "Data corrupted: invalid magic");
+    }
+
+    #[test]
+    fn storage_error_io_from_std_io_error() {
+        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
+        let storage_err: StorageError = io_err.into();
+        assert!(storage_err.to_string().contains("file missing"));
+    }
+
+    #[test]
+    fn storage_error_converts_to_wisp_error() {
+        let storage_err = StorageError::NotFound {
+            namespace: "ns".into(),
+            key: "k".into(),
+        };
+        let wisp_err: WispError = storage_err.into();
+        assert!(matches!(wisp_err, WispError::Storage(StorageError::NotFound { .. })));
+    }
+}
diff --git a/src/storage/mod.rs b/src/storage/mod.rs
index cd2f068..f592591 100644
--- a/src/storage/mod.rs
+++ b/src/storage/mod.rs
@@ -150,71 +150,71 @@ pub async fn delete_checkpoint(store: &dyn Store, name: &str) -> Result<()> {
 
 /// 保存元素快照。
 pub async fn save_element(
     store: &dyn Store,
     url: &str,
     key: &str,
     row: &ElementSnapshotRow,
 ) -> Result<()> {
     let composite = format!("{url}|{key}");
     let bytes = serde_json::to_vec(row).map_err(|e| {
-        WispError::Storage(StorageError::General(format!("serialize element: {e}")))
+        WispError::Storage(StorageError::Serialization(format!("serialize element: {e}")))
     })?;
     store.set(NS_ELEMENT, &composite, &bytes).await
 }
 
 /// 加载元素快照。
 pub async fn load_element(
     store: &dyn Store,
     url: &str,
     key: &str,
 ) -> Result<Option<ElementSnapshotRow>> {
     let composite = format!("{url}|{key}");
     store
         .get(NS_ELEMENT, &composite)
         .await?
         .map(|v| serde_json::from_slice(&v))
         .transpose()
-        .map_err(|e| WispError::Storage(StorageError::General(format!("parse element: {e}"))))
+        .map_err(|e| WispError::Storage(StorageError::Corrupted(format!("parse element: {e}"))))
 }
 
 // === Response Cache ===
 
 /// 保存响应缓存。
 pub async fn save_response(
     store: &dyn Store,
     method: &str,
     url: &str,
     resp: &CachedResponse,
 ) -> Result<()> {
     let composite = format!("{method}|{url}");
     let bytes = serde_json::to_vec(resp).map_err(|e| {
-        WispError::Storage(StorageError::General(format!("serialize response: {e}")))
+        WispError::Storage(StorageError::Serialization(format!("serialize response: {e}")))
     })?;
     store
         .set_with_ttl(NS_RESPONSE, &composite, &bytes, resp.ttl)
         .await
 }
 
 /// 加载响应缓存。
 pub async fn load_response(
     store: &dyn Store,
     method: &str,
     url: &str,
 ) -> Result<Option<CachedResponse>> {
     let composite = format!("{method}|{url}");
     store
         .get(NS_RESPONSE, &composite)
         .await?
         .map(|v| serde_json::from_slice(&v))
         .transpose()
-        .map_err(|e| WispError::Storage(StorageError::General(format!("parse response: {e}"))))
+        .map_err(|e| WispError::Storage(StorageError::Corrupted(format!("parse response: {e}"))))
 }
 
 /// 删除响应缓存。
 pub async fn delete_response(store: &dyn Store, method: &str, url: &str) -> Result<()> {
     let composite = format!("{method}|{url}");
     store.delete(NS_RESPONSE, &composite).await
 }
 
 // ============================================================================
 // 测试：自由函数 + MockStore
