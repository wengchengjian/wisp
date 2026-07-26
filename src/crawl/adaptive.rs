//! 自适应解析跟踪器 — 持久化元素快照。
//!
//! ARCH: 从 parser/adaptive.rs 迁移。parser 只产出 ElementSnapshot 值对象，
//! 持久化职责由 AdaptiveTracker 承担，消除 parser 对 storage 的依赖。

use std::sync::Arc;

use crate::error::Result;
use crate::parser::{relocate_with_snapshot, ElementSnapshot, Node, DEFAULT_TOLERANCE};
use crate::storage::{load_element, save_element, ElementSnapshotRow, Store};

/// 自适应解析跟踪器 — 持久化元素快照。
///
/// ARCH: 替代原 `parser::css_adaptive` 自由函数。持有 `Arc<dyn Store>`，
/// 提供 `css_adaptive` 方法：先 CSS 选择，失败则从存储加载快照重定位。
pub struct AdaptiveTracker {
    store: Arc<dyn Store>,
}

impl AdaptiveTracker {
    /// 创建跟踪器。
    #[must_use]
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    /// 自适应 CSS 选择：先 CSS，失败则从存储加载快照重定位。
    ///
    /// - `selector`: CSS 选择器（可能匹配也可能不匹配）
    /// - `key`: 元素稳定标识（用户定义，如 "product-name"）
    /// - `url`: 页面 URL（用于存储 key）
    /// - `auto_save`: 是否在成功后刷新快照
    /// - `tolerance`: 相似度阈值（0.0..1.0）
    pub async fn css_adaptive(
        &self,
        node: &Node,
        selector: &str,
        key: &str,
        url: &str,
        auto_save: bool,
        tolerance: f64,
    ) -> Result<Option<Node>> {
        // 1. Try CSS first
        if let Some(found) = node.select_one(selector) {
            if auto_save {
                let snap = ElementSnapshot::capture(&found);
                let row: ElementSnapshotRow = snap.into();
                let _ = save_element(self.store.as_ref(), url, key, &row).await;
            }
            return Ok(Some(found));
        }

        // 2. CSS failed - try relocate from saved snapshot
        let saved_row = match load_element(self.store.as_ref(), url, key).await? {
            Some(r) => r,
            None => return Ok(None),
        };
        let saved: ElementSnapshot = saved_row.into();
        let found = match relocate_with_snapshot(node, &saved, tolerance) {
            Some(n) => n,
            None => return Ok(None),
        };

        // 3. Auto-save new snapshot if relocated
        if auto_save {
            let snap = ElementSnapshot::capture(&found);
            let row: ElementSnapshotRow = snap.into();
            let _ = save_element(self.store.as_ref(), url, key, &row).await;
        }

        Ok(Some(found))
    }

    /// 使用默认 tolerance 的便捷方法。
    pub async fn css_adaptive_default(
        &self,
        node: &Node,
        selector: &str,
        key: &str,
        url: &str,
        auto_save: bool,
    ) -> Result<Option<Node>> {
        self.css_adaptive(node, selector, key, url, auto_save, DEFAULT_TOLERANCE)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStore;

    fn make_doc() -> Node {
        Node::from_html(
            r#"<html><body>
                <div class="item" id="main">Hello World</div>
            </body></html>"#,
        )
    }

    fn make_store() -> Arc<dyn Store> {
        Arc::new(MemoryStore::default())
    }

    #[tokio::test]
    async fn css_adaptive_returns_node_when_css_matches() {
        let tracker = AdaptiveTracker::new(make_store());
        let doc = make_doc();

        let found = tracker
            .css_adaptive(
                &doc,
                "#main",
                "main-key",
                "https://example.com",
                true,
                DEFAULT_TOLERANCE,
            )
            .await
            .expect("css_adaptive 应成功");

        assert!(found.is_some());
        assert_eq!(found.unwrap().text(), "Hello World");
    }

    #[tokio::test]
    async fn css_adaptive_returns_none_when_no_css_and_no_snapshot() {
        let tracker = AdaptiveTracker::new(make_store());
        let doc = make_doc();

        let found = tracker
            .css_adaptive(
                &doc,
                "#nonexistent",
                "missing-key",
                "https://example.com",
                false,
                DEFAULT_TOLERANCE,
            )
            .await
            .expect("css_adaptive 应成功");

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn css_adaptive_relocates_from_saved_snapshot() {
        let store = make_store();
        let tracker = AdaptiveTracker::new(Arc::clone(&store));

        // 第一次：CSS 匹配 #main，auto_save 保存快照
        let doc1 = make_doc();
        let found1 = tracker
            .css_adaptive(
                &doc1,
                "#main",
                "relocate-key",
                "https://example.com",
                true,
                DEFAULT_TOLERANCE,
            )
            .await
            .expect("css_adaptive 应成功");
        assert!(found1.is_some());

        // 第二次：CSS 不匹配（#missing-id），从存储加载快照重定位到 #main
        let doc2 = make_doc();
        let found2 = tracker
            .css_adaptive(
                &doc2,
                "#missing-id",
                "relocate-key",
                "https://example.com",
                false,
                DEFAULT_TOLERANCE,
            )
            .await
            .expect("css_adaptive 应成功");
        assert!(found2.is_some());
        assert_eq!(found2.unwrap().text(), "Hello World");
    }

    #[tokio::test]
    async fn css_adaptive_default_uses_default_tolerance() {
        let tracker = AdaptiveTracker::new(make_store());
        let doc = make_doc();

        let found = tracker
            .css_adaptive_default(&doc, "#main", "k", "https://x", false)
            .await
            .expect("css_adaptive_default 应成功");
        assert!(found.is_some());
    }
}
