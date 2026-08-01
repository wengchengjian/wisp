//! HTML fragment 解析。

use super::*;

fn fragment_inner_tag(html: &str) -> String {
    html.trim_start()
        .to_lowercase()
        .trim_start_matches('<')
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect()
}

fn is_table_fragment(tag: &str) -> bool {
    matches!(
        tag,
        "td" | "tr" | "th" | "thead" | "tbody" | "tfoot" | "caption" | "colgroup" | "col"
    )
}

/// 表格元素片段需要包裹 `<table>` 后用 `Document::parse` 解析，因为 HTML5 规范下
/// 这些表格元素在 `<body>` context 中不合法，html5ever 会丢弃标签（只保留文本内容）。
fn table_fragment_node(html: &str, tag: &str) -> Node {
    let wrapped = format!("<table>{}</table>", html);
    let doc = Document::from_html(&wrapped);
    let root_id = doc
        .html
        .query_selector(tag)
        .ok()
        .and_then(|nodes| nodes.into_iter().next().map(|n| n.id()))
        .unwrap_or_else(|| {
            doc.html
                .html_element()
                .map(|n| n.id())
                .unwrap_or_else(|| doc.html.root().id())
        });
    Node {
        doc,
        node_id: root_id,
    }
}

fn normal_fragment_node(html: &str) -> Node {
    let doc = Document::from_fragment(html);
    let root_id = doc
        .html
        .html_element()
        .and_then(|ctx| {
            ctx.children()
                .into_iter()
                .find(|c| c.is_element())
                .map(|c| c.id())
                .or(Some(ctx.id()))
        })
        .unwrap_or_else(|| doc.html.root().id());
    Node {
        doc,
        node_id: root_id,
    }
}

impl Node {
    /// Parse an HTML fragment.
    ///
    /// 普通元素片段用 `Document::parse_fragment`（保留片段语义，不创建 `<html><head><body>` 结构）。
    /// 表格元素片段（`<td>/<tr>/<th>/<thead>/<tbody>/<tfoot>/<caption>/<colgroup>/<col>`）
    /// 需要包裹 `<table>` 后用 `Document::parse` 解析；包裹后 html5ever 会规范化为
    /// `<table><tbody><tr><td>...</td></tr></tbody></table>`，保留表格元素标签，
    /// 然后用选择器深入找到实际的片段元素。
    ///
    /// 注意：裸文本/注释片段没有可用的元素子节点时会回退到 context 元素
    /// （此时 `tag()` 返回 `div`，可能不是用户期望的结果）。
    pub fn from_fragment(html: &str) -> Self {
        let inner_tag = fragment_inner_tag(html);
        if is_table_fragment(&inner_tag) {
            table_fragment_node(html, &inner_tag)
        } else {
            normal_fragment_node(html)
        }
    }
}
