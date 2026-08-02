//! AdaptiveTracker 实现。

use std::sync::Arc;

use wisp_core::error::Result;
use wisp_parser::{DEFAULT_TOLERANCE, ElementSnapshot, Node, relocate_with_snapshot};
use wisp_storage::{Store, load_element, save_element};

use super::convert::{row_to_snapshot, snapshot_to_row};

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

    async fn save_snapshot(&self, node: &Node, key: &str, url: &str) {
        let snap = ElementSnapshot::capture(node);
        let row = snapshot_to_row(snap, chrono::Utc::now().timestamp());
        if let Err(e) = save_element(self.store.as_ref(), url, key, &row).await {
            tracing::warn!("自适应快照保存失败: {}", e);
        }
    }

    async fn try_css(
        &self,
        node: &Node,
        selector: &str,
        key: &str,
        url: &str,
        auto_save: bool,
    ) -> Result<Option<Node>> {
        let Some(found) = node.select_one(selector) else {
            return Ok(None);
        };
        if auto_save {
            self.save_snapshot(&found, key, url).await;
        }
        Ok(Some(found))
    }

    async fn relocate_from_saved(
        &self,
        node: &Node,
        key: &str,
        url: &str,
        tolerance: f64,
        auto_save: bool,
    ) -> Result<Option<Node>> {
        let saved_row = match load_element(self.store.as_ref(), url, key).await? {
            Some(r) => r,
            None => return Ok(None),
        };
        let saved = row_to_snapshot(saved_row);
        let Some(found) = relocate_with_snapshot(node, &saved, tolerance) else {
            return Ok(None);
        };
        if auto_save {
            self.save_snapshot(&found, key, url).await;
        }
        Ok(Some(found))
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
        if let Some(found) = self.try_css(node, selector, key, url, auto_save).await? {
            return Ok(Some(found));
        }
        self.relocate_from_saved(node, key, url, tolerance, auto_save)
            .await
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
