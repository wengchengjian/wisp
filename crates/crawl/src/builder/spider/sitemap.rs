//! Sitemap 预设。

use regex::Regex;
use std::sync::LazyLock;

use super::SpiderBuilder;
use crate::Request;

/// Sitemap `<loc>` 提取正则：匹配 `<loc>URL</loc>` 中的 URL（允许前后空白）。
static RE_SITEMAP_LOC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<loc>\s*(.*?)\s*</loc>").unwrap());

impl SpiderBuilder {
    /// 预设：Sitemap 爬虫。
    ///
    /// 自动解析 sitemap.xml，提取 `<loc>` URL，follow 到指定 label 的 handler。
    ///
    /// # 示例
    /// ```
    /// use wisp_crawl::SpiderBuilder;
    /// use wisp_parser::ResponseExt;
    ///
    /// let spider = SpiderBuilder::sitemap("my_spider", vec!["https://x.com/sitemap.xml".into()], "content")
    ///     .on("content", |resp| async move {
    ///         (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
    ///     })
    ///     .build();
    /// ```
    pub fn sitemap(name: &str, sitemap_urls: Vec<String>, content_label: &str) -> Self {
        let label = content_label.to_string();
        SpiderBuilder::new(name)
            .start_urls(sitemap_urls)
            .on("default", move |resp| {
                let label = label.clone();
                async move {
                    let text = resp.text().unwrap_or_default();
                    let follows: Vec<Request> = RE_SITEMAP_LOC
                        .captures_iter(&text)
                        .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
                        .filter(|u| !u.is_empty())
                        .map(|url| Request::get(&url).with_callback(&label))
                        .collect();
                    (vec![], follows)
                }
            })
    }
}
