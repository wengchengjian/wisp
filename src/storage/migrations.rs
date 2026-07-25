//! SQLite schema migrations for the unified KV store.

/// 单表 KV schema。所有命名空间共享一张表。
pub const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS kv (
    namespace  TEXT NOT NULL,
    key        TEXT NOT NULL,
    value      BLOB NOT NULL,
    ttl_secs   INTEGER,                -- NULL = 永不过期
    cached_at  INTEGER NOT NULL,       -- Unix 秒，写入时刻
    PRIMARY KEY (namespace, key)
);
CREATE INDEX IF NOT EXISTS idx_kv_namespace ON kv(namespace);
"#;
