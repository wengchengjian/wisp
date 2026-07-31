//! Document: 共享所有权的 HTML 文档容器。
//!
//! 包含 woven_html::Document（CSS 查询）。
//! Node 通过 Arc<Document> 共享文档，select() 返回的 Node 引用同一文档的树中位置。

use std::sync::Arc;
use woven_html::Document as HtmlDocument;

/// 共享的 HTML 文档。woven-html 树用于 CSS 查询和 DOM 导航。
///
/// Document 总是通过 `Arc<Document>` 共享，内部直接持有 `Arc<HtmlDocument>`。
pub struct Document {
    /// woven-html 解析的 HTML 树
    pub(crate) html: Arc<HtmlDocument>,
}

impl Document {
    /// 从 HTML 字符串创建文档。
    ///
    /// 用 `HtmlDocument::parse` 解析，会应用 HTML5 结构规则
    /// （如把 `<td>/<tr>` 等表格元素强制包裹 `<table><tbody><tr>`）。
    /// 适合完整 HTML 文档；若需保留片段语义，用 `from_fragment`。
    pub fn from_html(html: &str) -> Arc<Self> {
        Self::from_html_owned(html.to_string())
    }

    /// 从已解码的 HTML 字符串创建文档（直接移交所有权，避免整页复制）。
    pub fn from_html_owned(html: String) -> Arc<Self> {
        let parsed = HtmlDocument::parse(html).expect("woven-html: HTML 文档解析失败");
        Arc::new(Self { html: parsed })
    }

    /// 从 HTML 片段创建文档（不应用完整文档结构规则）。
    ///
    /// 用 `HtmlDocument::parse_fragment(html, "div")` 解析，context 元素（div）作为
    /// fragment 的挂载点。适合解析独立的普通元素片段。
    pub fn from_fragment(html: &str) -> Arc<Self> {
        let parsed = HtmlDocument::parse_fragment(html.to_string(), "div")
            .expect("woven-html: HTML 片段解析失败");
        Arc::new(Self { html: parsed })
    }
}
