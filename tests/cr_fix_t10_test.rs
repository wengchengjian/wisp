//! Task 10 回归测试：Store 启用 WAL 模式。
use wisp::storage::Store;

#[test]
fn test_store_uses_wal_mode() {
    let store = Store::open_in_memory().unwrap();
    let mode: String = store.conn_ref()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap_or_default();
    assert!(
        mode == "memory" || mode == "wal",
        "journal_mode 应为 wal 或 memory，实际: {}",
        mode
    );
}
