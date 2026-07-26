//! Document: 共享所有权的 HTML 文档容器。
//!
//! 包含 scraper::Html（CSS 查询）。
//! Node 通过 Arc<Document> 共享文档，select() 返回的 Node 引用同一文档的树中位置。

use scraper::Html;
use std::sync::Arc;

/// 共享的 HTML 文档。scraper 树用于 CSS 查询和 DOM 导航。
///
/// Document 总是通过 `Arc<Document>` 共享，内部直接持有 `Html`（无需再包一层 Arc）。
pub struct Document {
    /// scraper 解析的 HTML 树（html5ever 容错）
    pub(crate) html: Html,
}

impl Document {
    /// 从 HTML 字符串创建文档。
    ///
    /// 用 `Html::parse_document` 解析，会应用 HTML5 结构规则
    /// （如把 `<td>/<tr>` 等表格元素强制包裹 `<table><tbody><tr>`）。
    /// 适合完整 HTML 文档；若需保留片段语义（不包裹 table），用 `from_fragment`。
    #[must_use]
    pub fn from_html(html: &str) -> Arc<Self> {
        let parsed = Html::parse_document(html);
        Arc::new(Self { html: parsed })
    }

    /// 从 HTML 片段创建文档（不应用 HTML5 结构规则）。
    ///
    /// 用 `Html::parse_fragment` 解析，避免 `<td>/<tr>/<thead>/<tbody>/<th>/<caption>`
    /// 等表格元素被强制包裹 `<table><tbody><tr>`，保留片段语义。
    /// 适合解析独立的元素片段（如 `<td>cell</td>` 应保持 tag 为 `td`）。
    #[must_use]
    pub fn from_fragment(html: &str) -> Arc<Self> {
        let parsed = Html::parse_fragment(html);
        Arc::new(Self { html: parsed })
    }
}
