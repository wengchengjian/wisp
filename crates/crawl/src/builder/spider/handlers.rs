//! 页面/链接/内容 handler 注册。

use super::{Handler, SpiderBuilder};
use crate::page::Page;
use serde_json::Value;
use std::sync::Arc;
use wisp_parser::Node;

impl SpiderBuilder {
    /// 注册页面 handler：接收 [`Page`]，内部自动解析一次并收集 items/follows。
    ///
    /// 与 [`Self::on`] 不同，handler 是同步的，返回 `Page` 而非
    /// `(items, follows)`；`build()` 时统一转换为引擎所需格式。
    ///
    /// # 示例
    /// ```rust,no_run
    /// use wisp_crawl::SpiderBuilder;
    ///
    /// let spider = SpiderBuilder::new("quotes")
    ///     .start_urls(vec!["https://quotes.toscrape.com/"])
    ///     .on_page("default", |mut page| {
    ///         page.follow_links(&[".quote a"], "detail", |_page, _idx, a| {
    ///             serde_json::json!({ "title": a.text().trim() })
    ///         });
    ///         page
    ///     })
    ///     .on_page("detail", |mut page| {
    ///         page.item(serde_json::json!({ "title": page.meta_str("title") }));
    ///         page
    ///     })
    ///     .build();
    /// ```
    pub fn on_page<F>(mut self, label: &str, handler: F) -> Self
    where
        F: Fn(Page) -> Page + Send + Sync + 'static,
    {
        let boxed: Handler = Arc::new(move |resp| {
            let page = Page::new(resp);
            let (items, follows) = handler(page).finish();
            Box::pin(async move { (items, follows) })
        });
        self.handlers.insert(label.to_string(), boxed);
        self
    }

    /// 注册纯链接提取 handler：匹配第一个非空选择器，follow 到 `callback`。
    ///
    /// 适合“列表页只负责发现链接”的流程；需要同时产出 item 或诊断时用
    /// [`Self::on_page`]。
    pub fn on_links<F>(self, label: &str, selectors: &[&str], callback: &str, meta_for: F) -> Self
    where
        F: Fn(&Page, usize, &Node) -> Value + Send + Sync + 'static,
    {
        self.on_links_n(label, selectors, callback, usize::MAX, meta_for)
    }

    /// 注册纯链接提取 handler，最多跟随前 `limit` 个链接。
    pub fn on_links_n<F>(
        self,
        label: &str,
        selectors: &[&str],
        callback: &str,
        limit: usize,
        meta_for: F,
    ) -> Self
    where
        F: Fn(&Page, usize, &Node) -> Value + Send + Sync + 'static,
    {
        let selectors: Vec<String> = selectors.iter().map(|s| s.to_string()).collect();
        let callback = callback.to_string();
        self.on_page(label, move |mut page| {
            let selectors: Vec<&str> = selectors.iter().map(String::as_str).collect();
            page.follow_links_n(&selectors, &callback, limit, &meta_for);
            page
        })
    }

    /// 注册内容页 handler：从页面提取正文，合并 meta 字段并产出 item。
    ///
    /// item 由请求 meta + `content` + `url` 组成，适合“章节页收集正文”的流程。
    /// `clean` 接收原始文本并返回清洗后的文本。
    pub fn on_content<F>(self, label: &str, selectors: &[&str], clean: F) -> Self
    where
        F: Fn(String) -> String + Send + Sync + 'static,
    {
        let selectors: Vec<String> = selectors.iter().map(|s| s.to_string()).collect();
        self.on_page(label, move |mut page| {
            let selectors: Vec<&str> = selectors.iter().map(String::as_str).collect();
            let text = (&clean)(page.content_text(&selectors));
            let mut item = match page.meta_owned() {
                Value::Object(map) => Value::Object(map),
                _ => Value::Object(serde_json::Map::new()),
            };
            if let Value::Object(ref mut map) = item {
                map.insert("content".to_string(), Value::String(text));
                map.insert("url".to_string(), Value::String(page.url().to_string()));
            }
            page.item_value(item);
            page
        })
    }
}
