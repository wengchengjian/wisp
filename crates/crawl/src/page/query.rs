//! 页面文档查询与基本信息。

use super::*;
use wisp_parser::NodeList;

impl Page {
    /// 原始响应（不可再调用其 `parse()` 系列方法）。
    pub fn resp(&self) -> &Response {
        &self.resp
    }

    /// 页面文档节点（轻量句柄，可任意次获取）。
    pub fn doc(&self) -> Node {
        self.doc.clone()
    }

    /// CSS 选择器查询（基于已解析文档，可任意次调用）。
    pub fn css(&self, selector: &str) -> NodeList {
        self.doc.select(selector)
    }

    /// CSS 选择器查询第一个匹配元素。
    pub fn select_one(&self, selector: &str) -> Option<Node> {
        self.doc.select_one(selector)
    }

    /// 按选择器列表提取正文文本；使用第一个非空选择器。
    ///
    /// 自动 trim 每行并过滤空行，适合“章节页取正文”的常见流程。
    pub fn content_text(&self, selectors: &[&str]) -> String {
        for selector in selectors {
            if let Some(node) = self.doc.select_one(selector) {
                let text = node.text();
                let mut out = String::with_capacity(text.len());
                for line in text.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        out.push_str(trimmed);
                        out.push('\n');
                    }
                }
                if out.ends_with('\n') {
                    out.pop();
                }
                return out;
            }
        }
        String::new()
    }

    /// 页面 URL。
    pub fn url(&self) -> &str {
        &self.resp.url
    }

    /// HTTP 状态码。
    pub fn status(&self) -> u16 {
        self.resp.status
    }
}
