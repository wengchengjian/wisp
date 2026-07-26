//! 文件系统存储后端。每条 entry 一个文件，子目录隔离 namespace。
//!
//! 所有同步 fs I/O 用 `tokio::task::spawn_blocking` 包装移出 async worker。
//! 写入采用 `tempfile::NamedTempFile::persist` 原子替换：先写到同目录下随机命名
//! 的临时文件，再 `rename` 到目标路径。POSIX `rename` 原子，保证并发写同 key 时
//! 文件内容不会交错损坏（最后一个写者决定最终内容）。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tempfile::NamedTempFile;

use super::Store;
use crate::error::{Result, StorageError, WispError};

/// 将任意 key sanitize 为安全文件名组件。
///
/// 替换文件系统非法字符（`/` `\` `:` `*` `?` `"` `<` `>` `|`）为 `_`，
/// 处理 Windows 保留名（CON/PRN/AUX/NUL/COM1-9/LPT1-9）加 `wisp_` 前缀。
/// 截断至 200 字符防止文件名过长。
fn sanitize_key(key: &str) -> String {
    let s: String = key
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let upper = s.to_uppercase();
    let is_reserved = matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    let base = if is_reserved { format!("wisp_{s}") } else { s };
    base.chars().take(200).collect()
}

/// 文件系统存储后端。
///
/// 目录结构：
/// ```text
/// <root>/
/// ├── checkpoint/<sanitized_key>
/// ├── element/<sanitized_key>
/// └── response/<sanitized_key>
/// ```
///
/// TTL 实现：在文件内容前缀附 8 字节 `expires_at`（Unix 秒，big-endian）。
/// `get` 时检查过期，过期则删除文件并返回 `None`。
///
/// 性能：每条 entry 一个文件，适合 checkpoint/element（数据量小）；
///       response cache 不建议使用 FileStore（Engine 默认用 MemoryStore）。
pub struct FileStore {
    root: PathBuf,
}

impl FileStore {
    /// 自定义根目录。会自动创建。
    #[must_use]
    pub fn with_dir(root: PathBuf) -> Self {
        let _ = fs::create_dir_all(&root); // 容忍已存在
        Self { root }
    }
}

impl Default for FileStore {
    /// 默认根目录 `./wisp-data/`（相对当前工作目录）。
    fn default() -> Self {
        Self::with_dir(PathBuf::from("./wisp-data"))
    }
}

/// 序列化带 TTL 的 entry：`[expires_at: 8 bytes BE i64][value...]`。
/// `expires_at == 0` 表示永不过期；非零为 Unix 毫秒时间戳。
fn pack_with_ttl(value: &[u8], ttl: Option<Duration>) -> Vec<u8> {
    let expires_at: i64 = match ttl {
        None => 0,
        Some(d) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |n| n.as_millis() as i64);
            now + d.as_millis() as i64
        }
    };
    let mut buf = expires_at.to_be_bytes().to_vec();
    buf.extend_from_slice(value);
    buf
}

/// 反序列化，返回 `(value, expired)`。文件损坏返回 `None`。
fn unpack_and_check(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 8 {
        return None;
    }
    let expires_at = i64::from_be_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);
    if expires_at == 0 {
        return Some(data[8..].to_vec());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |n| n.as_millis() as i64);
    if now > expires_at {
        return None; // 已过期
    }
    Some(data[8..].to_vec())
}

/// 原子写入：先写到同目录下随机命名的临时文件，再 `rename` 到目标路径。
///
/// POSIX `rename` 原子，保证并发写同 path 时文件内容不会交错损坏。
/// 失败时临时文件会被 NamedTempFile drop 自动清理。
fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut tmp, data)?;
    tmp.persist(path)
        .map_err(|e| std::io::Error::other(format!("persist failed: {e}")))?;
    Ok(())
}

#[async_trait]
impl Store for FileStore {
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let root = self.root.clone();
        let namespace = namespace.to_string();
        let key = key.to_string();
        let value = value.to_vec();
        tokio::task::spawn_blocking(move || {
            let path = root.join(&namespace).join(sanitize_key(&key));
            let packed = pack_with_ttl(&value, None);
            atomic_write(&path, &packed)
                .map_err(|e| WispError::Storage(StorageError::General(format!("write: {e}"))))?;
            Ok(())
        })
        .await
        .map_err(|e| {
            WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}")))
        })?
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let root = self.root.clone();
        let namespace = namespace.to_string();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let path = root.join(&namespace).join(sanitize_key(&key));
            let mut file = match fs::File::open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => {
                    return Err(WispError::Storage(StorageError::General(format!(
                        "open: {e}"
                    ))))
                }
            };
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .map_err(|e| WispError::Storage(StorageError::General(format!("read: {e}"))))?;
            if let Some(v) = unpack_and_check(&buf) {
                Ok(Some(v))
            } else {
                // 过期或损坏：惰性删除
                let _ = fs::remove_file(&path);
                Ok(None)
            }
        })
        .await
        .map_err(|e| {
            WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}")))
        })?
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let root = self.root.clone();
        let namespace = namespace.to_string();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let path = root.join(&namespace).join(sanitize_key(&key));
            match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(WispError::Storage(StorageError::General(format!(
                    "delete: {e}"
                )))),
            }
        })
        .await
        .map_err(|e| {
            WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}")))
        })?
    }

    async fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let root = self.root.clone();
        let namespace = namespace.to_string();
        let key = key.to_string();
        let value = value.to_vec();
        tokio::task::spawn_blocking(move || {
            let path = root.join(&namespace).join(sanitize_key(&key));
            let packed = pack_with_ttl(&value, ttl);
            atomic_write(&path, &packed)
                .map_err(|e| WispError::Storage(StorageError::General(format!("write: {e}"))))?;
            Ok(())
        })
        .await
        .map_err(|e| {
            WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}")))
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_store() -> (FileStore, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let store = FileStore::with_dir(dir.path().to_path_buf());
        (store, dir)
    }

    #[tokio::test]
    async fn checkpoint_roundtrip() {
        let (store, _d) = make_store();
        store.set("checkpoint", "spider1", b"state").await.unwrap();
        assert_eq!(
            store.get("checkpoint", "spider1").await.unwrap().unwrap(),
            b"state"
        );
        store.delete("checkpoint", "spider1").await.unwrap();
        assert!(store.get("checkpoint", "spider1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn ttl_expiry() {
        let (store, _d) = make_store();
        store
            .set_with_ttl("ns", "k", b"v", Some(Duration::from_millis(1)))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(store.get("ns", "k").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn ttl_none_never_expires() {
        let (store, _d) = make_store();
        store
            .set_with_ttl("ns", "k", b"forever", None)
            .await
            .unwrap();
        assert_eq!(store.get("ns", "k").await.unwrap().unwrap(), b"forever");
    }

    #[tokio::test]
    async fn delete_missing_is_ok() {
        let (store, _d) = make_store();
        store.delete("ns", "nonexistent").await.unwrap();
    }

    #[tokio::test]
    async fn namespace_isolation() {
        let (store, _d) = make_store();
        store.set("ns1", "key", b"a").await.unwrap();
        store.set("ns2", "key", b"b").await.unwrap();
        assert_eq!(store.get("ns1", "key").await.unwrap().unwrap(), b"a");
        assert_eq!(store.get("ns2", "key").await.unwrap().unwrap(), b"b");
    }

    #[test]
    fn sanitize_key_replaces_separators() {
        assert!(sanitize_key("a/b").contains('_'));
        assert!(sanitize_key("a\\b").contains('_'));
        assert!(sanitize_key("a:b").contains('_'));
        // Windows 保留名加前缀
        assert!(sanitize_key("CON").starts_with("wisp_"));
    }

    /// 验证 spawn_blocking 并发写入：10 个 namespace × 10 次 set 应快速完成。
    #[tokio::test]
    async fn test_file_store_async_concurrent_writes() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        use tempfile::TempDir;

        let tmp = TempDir::new().expect("create temp dir");
        let store = Arc::new(FileStore::with_dir(tmp.path().to_path_buf()));

        let mut handles = Vec::new();
        let start = Instant::now();
        for ns_idx in 0..10 {
            let store = Arc::clone(&store);
            let handle = tokio::spawn(async move {
                let ns = format!("ns_{ns_idx}");
                for i in 0..10 {
                    store
                        .set(&ns, &format!("k{i}"), b"v")
                        .await
                        .expect("set should succeed");
                }
            });
            handles.push(handle);
        }
        for handle in handles {
            handle.await.unwrap();
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(3),
            "并发写入应 < 3s，实际 {elapsed:?}"
        );
    }
}
