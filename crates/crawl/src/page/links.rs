//! 链接提取与 follow 请求生成。

use super::*;
use wisp_parser::Node;

impl Page {
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
        self.follow_links_filtered_n(selectors, callback, limit, |_| true, meta_for)
    }

    /// 按选择器提取链接并用 URL predicate 过滤，符合条件才生成 follow 请求。
    pub fn follow_links_filtered(
        &mut self,
        selectors: &[&str],
        callback: &str,
        predicate: impl Fn(&str) -> bool,
        meta_for: impl Fn(&Page, usize, &Node) -> Value,
    ) -> &mut Self {
        self.follow_links_filtered_n(selectors, callback, usize::MAX, predicate, meta_for)
    }

    fn follow_links_filtered_n(
        &mut self,
        selectors: &[&str],
        callback: &str,
        limit: usize,
        predicate: impl Fn(&str) -> bool,
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
                let meta = meta_for(self, idx, a);
                if let Some(req) = self.resp.follow_meta(&href, meta)
                    && predicate(&req.url)
                {
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
}
