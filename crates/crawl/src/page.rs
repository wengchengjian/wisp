//! Page: 单页面处理上下文。
//!
//! 收敛 Spider handler 中最常见的重复流程：解析一次、在文档上多次查询、
//! 收集 item、从链接生成 follow 请求，最后统一产出 `(items, follows)`。

use serde::Serialize;
use serde_json::Value;

use wisp_fetcher::{Request, Response};
use wisp_parser::{Node, NodeList, ResponseExt};

/// 单页面处理上下文。
///
/// 由 [`crate::SpiderBuilder::on_page`] 创建，构造时解析一次 HTML，
/// 之后可通过 [`Page::doc`] / [`Page::css`] 任意次查询，无需担心
/// `Response::parse()` 的“仅一次”限制。
pub struct Page {
    resp: Response,
    doc: Node,
    items: Vec<Value>,
    follows: Vec<Request>,
}

impl Page {
    /// 从响应创建页面上下文，并立即解析 HTML。
    ///
    /// # Panics
    /// 传入的 `Response` 必须尚未调用过 `parse()`/`css()` 等便捷方法。
    pub fn new(resp: Response) -> Self {
        let doc = resp.parse();
        Self {
            resp,
            doc,
            items: Vec::new(),
            follows: Vec::new(),
        }
    }

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
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(trimmed);
                    }
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

    /// 请求 meta。
    pub fn meta(&self) -> &Value {
        &self.resp.request.meta
    }

    /// 取出请求 meta（避免内容页把整份 meta 克隆进 item）。
    pub fn meta_owned(&mut self) -> Value {
        std::mem::take(&mut self.resp.request.meta)
    }

    /// 读取 meta 字符串字段，缺失时返回空字符串。
    pub fn meta_str(&self, key: &str) -> String {
        self.meta_str_or(key, "")
    }

    /// 读取 meta 字符串字段，缺失时使用默认值。
    pub fn meta_str_or(&self, key: &str, default: &str) -> String {
        self.meta()
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_string()
    }

    /// 读取 meta 数字字段，缺失时返回 0。
    pub fn meta_u64(&self, key: &str) -> u64 {
        self.meta().get(key).and_then(Value::as_u64).unwrap_or(0)
    }

    /// 收集一个 item（自动序列化为 JSON）。
    pub fn item(&mut self, item: impl Serialize) -> &mut Self {
        self.items
            .push(serde_json::to_value(item).unwrap_or_default());
        self
    }

    /// 直接收集一个已序列化的 JSON value（避免 `Value` 再次经过序列化克隆）。
    pub fn item_value(&mut self, value: Value) -> &mut Self {
        self.items.push(value);
        self
    }

    /// 已收集的 items。
    pub fn items(&self) -> &[Value] {
        &self.items
    }

    /// 跟随链接到指定 callback（不带 meta）。
    pub fn follow(&mut self, href: &str, callback: &str) -> &mut Self {
        if let Some(req) = self.resp.follow_with(href, callback) {
            self.follows.push(req);
        }
        self
    }

    /// 跟随链接到指定 callback（带 meta）。
    pub fn follow_meta(&mut self, href: &str, callback: &str, meta: Value) -> &mut Self {
        if let Some(req) = self.resp.follow_meta(href, meta) {
            self.follows.push(req.with_callback(callback));
        }
        self
    }

    /// 按选择器列表提取链接并生成 follow 请求。
    ///
    /// 使用第一个非空选择器；`meta_for` 接收元素索引与链接元素，
    /// 返回该链接携带的 meta。无法解析为绝对 URL 或缺少 href/文本的链接会被忽略。
    pub fn follow_links(
        &mut self,
        selectors: &[&str],
        callback: &str,
        meta_for: impl Fn(&Page, usize, &Node) -> Value,
    ) -> &mut Self {
        self.follow_links_n(selectors, callback, usize::MAX, meta_for)
    }

    /// 按选择器列表提取链接并生成 follow 请求，最多跟随前 `limit` 个有效链接。
    ///
    /// 适用于“只抓列表页前 N 个业务实体”的语义，例如首页只取前 20 本书。
    pub fn follow_links_n(
        &mut self,
        selectors: &[&str],
        callback: &str,
        limit: usize,
        meta_for: impl Fn(&Page, usize, &Node) -> Value,
    ) -> &mut Self {
        let mut pending = Vec::new();
        for sel in selectors {
            let links = self.doc.select(sel);
            if links.is_empty() {
                continue;
            }
            let mut followed = 0usize;
            for (idx, a) in links.iter().enumerate() {
                if followed >= limit {
                    break;
                }
                let Some(href) = a.attr("href") else {
                    continue;
                };
                let title = a.text().trim().to_string();
                if href.is_empty() || title.is_empty() {
                    continue;
                }
                if let Some(req) = self.resp.follow_meta(&href, meta_for(self, idx, a)) {
                    pending.push(req.with_callback(callback));
                    followed += 1;
                }
            }
            break;
        }
        self.follows.extend(pending);
        self
    }

    /// 已生成的 follows。
    pub fn follows(&self) -> &[Request] {
        &self.follows
    }

    /// 完成处理，产出 `(items, follows)`。
    pub fn finish(self) -> (Vec<Value>, Vec<Request>) {
        (self.items, self.follows)
    }
}

impl From<Page> for (Vec<Value>, Vec<Request>) {
    fn from(page: Page) -> Self {
        page.finish()
    }
}
