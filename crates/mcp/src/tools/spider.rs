//! MCP SimpleSpider。

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::Arc;
use wisp_core::{Request, Response};
use wisp_crawl::{MaxPages, Spider, StopCondition};
use wisp_parser::Node;

pub(super) struct SimpleSpider {
    pub(super) css: String,
    pub(super) start_urls: Vec<String>,
    pub(super) max_pages: usize,
    pub(super) follow_pattern: Option<Regex>,
    pub(super) max_depth: u32,
    pub(super) allowed_domains: Vec<String>,
}

#[async_trait]
impl Spider for SimpleSpider {
    fn name(&self) -> &str {
        "mcp_simple"
    }
    fn start_urls(&self) -> Vec<String> {
        self.start_urls.clone()
    }
    async fn handle(&self, resp: Response) -> (Vec<Value>, Vec<Request>) {
        let text = resp.text().unwrap_or_default();
        let doc = Node::from_html(&text);
        let nodes = doc.select(&self.css);
        let items: Vec<Value> = nodes
            .iter()
            .map(|n| json!({"text": n.text(), "html": n.html()}))
            .collect();

        let page_url = resp.url.clone();
        let spider_name = resp
            .request
            .spider
            .clone()
            .unwrap_or_else(|| self.name().to_string());
        let current_depth = resp.request.depth;
        let mut follows = Vec::new();
        for link in doc.select("a[href]") {
            let Some(href) = link.attr("href") else {
                continue;
            };
            let Some(url) = wisp_core::utils::resolve_href(&page_url, &href) else {
                continue;
            };
            if let Some(ref re) = self.follow_pattern {
                if !re.is_match(&url) {
                    continue;
                }
            }
            if self.max_depth > 0 && current_depth + 1 > self.max_depth {
                continue;
            }
            if !self.allowed_domains.is_empty() {
                let host = url::Url::parse(&url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_string))
                    .unwrap_or_default();
                let allowed = self
                    .allowed_domains
                    .iter()
                    .any(|d| host == *d || host.ends_with(&format!(".{d}")));
                if !allowed {
                    continue;
                }
            }
            let mut req = Request::get(&url)
                .with_spider(spider_name.clone())
                .with_depth(current_depth + 1);
            if let Some(ref cb) = resp.request.callback {
                req = req.with_callback(cb.as_str());
            }
            follows.push(req);
        }
        (items, follows)
    }
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(MaxPages(self.max_pages))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    #[tokio::test]
    async fn follows_matching_links_with_depth_and_domain() {
        let spider = SimpleSpider {
            css: "p".into(),
            start_urls: vec!["https://example.com/".into()],
            max_pages: 10,
            follow_pattern: Some(Regex::new(r"^https://example\.com/blog/").unwrap()),
            max_depth: 1,
            allowed_domains: vec!["example.com".into()],
        };
        let resp = Response::from_browser(
            200,
            "https://example.com/".into(),
            r#"<a href="/blog/1">b</a><a href="/other">o</a><a href="https://evil.com/x">e</a>"#
                .into(),
            "t".into(),
            vec![],
            Request::get("https://example.com/"),
        );
        let (_items, follows) = spider.handle(resp).await;
        assert_eq!(follows.len(), 1);
        assert_eq!(follows[0].url, "https://example.com/blog/1");
        assert_eq!(follows[0].depth, 1);
        assert_eq!(follows[0].spider.as_deref(), Some("mcp_simple"));
    }

    #[tokio::test]
    async fn respects_max_depth() {
        let spider = SimpleSpider {
            css: "p".into(),
            start_urls: vec!["https://example.com/".into()],
            max_pages: 10,
            follow_pattern: None,
            max_depth: 1,
            allowed_domains: vec![],
        };
        let mut req = Request::get("https://example.com/");
        req.depth = 1;
        let resp = Response::from_browser(
            200,
            "https://example.com/".into(),
            r#"<a href="/next">x</a>"#.into(),
            "t".into(),
            vec![],
            req,
        );
        let (_items, follows) = spider.handle(resp).await;
        assert!(follows.is_empty(), "depth 1 + max_depth 1 不应继续跟随");
    }
}
