//! SQLite schema migrations for the unified storage layer.

/// SQL statements to create all tables for stage 1 (adaptive + checkpoint).
/// Session/cache tables are added in stage 4.
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
"#;
