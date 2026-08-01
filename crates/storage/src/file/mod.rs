//! 文件系统存储后端。每条 entry 一个文件，子目录隔离 namespace。
//!
//! 所有同步 fs I/O 用 `tokio::task::spawn_blocking` 包装移出 async worker。

mod io;
mod path;
mod ttl;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

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
/// 所有同步 I/O 都经 `spawn_blocking` 移出 async worker；同 key 并发写
/// 依赖文件系统原子性，与旧实现行为一致。
pub struct FileStore {
    root: PathBuf,
}

impl FileStore {
    /// 自定义根目录。会自动创建。
    pub fn with_dir(root: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&root); // 容忍已存在
        Self { root }
    }
}

impl Default for FileStore {
    /// 默认根目录 `./wisp-data/`（相对当前工作目录）。
    fn default() -> Self {
        Self::with_dir(PathBuf::from("./wisp-data"))
    }
}
