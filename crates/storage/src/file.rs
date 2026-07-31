//! 文件系统存储后端。每条 entry 一个文件，子目录隔离 namespace。

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use wisp_core::error::{Result, StorageError, WispError};
use super::Store;

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
        "CON" | "PRN" | "AUX" | "NUL"
        | "COM1" | "COM2" | "COM3" | "COM4" | "COM5"
        | "COM6" | "COM7" | "COM8" | "COM9"
        | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5"
        | "LPT6" | "LPT7" | "LPT8" | "LPT9"
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
/// 线程安全：单 `parking_lot::Mutex<()>` 保护并发写（文件级互斥简化实现）。
/// 性能：每条 entry 一个文件，适合 checkpoint/element（数据量小）；
///       response cache 不建议使用 FileStore（Engine 默认用 MemoryStore）。
pub struct FileStore {
    root: PathBuf,
    /// 全局写锁（简化实现；可优化为 per-namespace 锁）。
    write_lock: Mutex<()>,
}

impl FileStore {
    /// 自定义根目录。会自动创建。
    pub fn with_dir(root: PathBuf) -> Self {
        let _ = fs::create_dir_all(&root);  // 容忍已存在
        Self { root, write_lock: Mutex::new(()) }
    }

    fn path_for(&self, namespace: &str, key: &str) -> PathBuf {
        let sanitized = sanitize_key(key);
        self.root.join(namespace).join(sanitized)
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
            let now = SystemTime::now().duration_since(UNIX_EPOCH)
                .map(|n| n.as_millis() as i64)
                .unwrap_or(0);
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
    let now = SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|n| n.as_millis() as i64)
        .unwrap_or(0);
    if now > expires_at {
        return None;  // 已过期
    }
    Some(data[8..].to_vec())
}

impl Store for FileStore {
    fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let _guard = self.write_lock.lock();
        let path = self.path_for(namespace, key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| WispError::Storage(StorageError::General(format!("mkdir: {e}"))))?;
        }
        let packed = pack_with_ttl(value, None);
        fs::write(&path, packed)
            .map_err(|e| WispError::Storage(StorageError::General(format!("write: {e}"))))?;
        Ok(())
    }

    fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let path = self.path_for(namespace, key);
        let mut file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(WispError::Storage(StorageError::General(format!("open: {e}")))),
        };
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| WispError::Storage(StorageError::General(format!("read: {e}"))))?;
        match unpack_and_check(&buf) {
            Some(v) => Ok(Some(v)),
            None => {
                // 过期或损坏：惰性删除
                let _ = fs::remove_file(&path);
                Ok(None)
            }
        }
    }

    fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let path = self.path_for(namespace, key);
        match fs::remove_file(&path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(WispError::Storage(StorageError::General(format!("delete: {e}")))),
        }
    }

    fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let _guard = self.write_lock.lock();
        let path = self.path_for(namespace, key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| WispError::Storage(StorageError::General(format!("mkdir: {e}"))))?;
        }
        let packed = pack_with_ttl(value, ttl);
        fs::write(&path, packed)
            .map_err(|e| WispError::Storage(StorageError::General(format!("write: {e}"))))?;
        Ok(())
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

    #[test]
    fn checkpoint_roundtrip() {
        let (store, _d) = make_store();
        store.set("checkpoint", "spider1", b"state").unwrap();
        assert_eq!(store.get("checkpoint", "spider1").unwrap().unwrap(), b"state");
        store.delete("checkpoint", "spider1").unwrap();
        assert!(store.get("checkpoint", "spider1").unwrap().is_none());
    }

    #[test]
    fn ttl_expiry() {
        let (store, _d) = make_store();
        store.set_with_ttl("ns", "k", b"v", Some(Duration::from_millis(1))).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        assert!(store.get("ns", "k").unwrap().is_none());
    }

    #[test]
    fn ttl_none_never_expires() {
        let (store, _d) = make_store();
        store.set_with_ttl("ns", "k", b"forever", None).unwrap();
        assert_eq!(store.get("ns", "k").unwrap().unwrap(), b"forever");
    }

    #[test]
    fn delete_missing_is_ok() {
        let (store, _d) = make_store();
        store.delete("ns", "nonexistent").unwrap();
    }

    #[test]
    fn namespace_isolation() {
        let (store, _d) = make_store();
        store.set("ns1", "key", b"a").unwrap();
        store.set("ns2", "key", b"b").unwrap();
        assert_eq!(store.get("ns1", "key").unwrap().unwrap(), b"a");
        assert_eq!(store.get("ns2", "key").unwrap().unwrap(), b"b");
    }

    #[test]
    fn sanitize_key_replaces_separators() {
        assert!(sanitize_key("a/b").contains('_'));
        assert!(sanitize_key("a\\b").contains('_'));
        assert!(sanitize_key("a:b").contains('_'));
        // Windows 保留名加前缀
        assert!(sanitize_key("CON").starts_with("wisp_"));
    }
}
