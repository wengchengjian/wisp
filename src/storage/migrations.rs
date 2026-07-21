//! SQLite schema migrations for the unified storage layer.

/// SQL statements to create all tables for schema v1.
///
/// 包含：element_snapshots（自适应定位）、crawl_checkpoints（断点续爬）、
/// response_cache（开发模式 replay 缓存）。
pub const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS element_snapshots (
    url TEXT NOT NULL,
    key TEXT NOT NULL,
    tag TEXT,
    attrs TEXT,
    text_preview TEXT,
    ancestor_path TEXT,
    sibling_tags TEXT,
    position_in_parent INTEGER,
    parent_tag TEXT,
    parent_attrs TEXT,
    captured_at INTEGER,
    PRIMARY KEY (url, key)
);

CREATE TABLE IF NOT EXISTS crawl_checkpoints (
    spider_name TEXT PRIMARY KEY,
    state BLOB NOT NULL,
    saved_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS response_cache (
    url TEXT NOT NULL,
    method TEXT NOT NULL,
    status INTEGER,
    headers TEXT,            -- JSON
    body BLOB,
    cached_at INTEGER,
    PRIMARY KEY (url, method)
);
"#;
