//! MCP SimpleSpider。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use wisp_core::{Request, Response};
use wisp_crawl::{MaxPages, Spider, StopCondition};
use wisp_parser::Node;

pub(super) struct SimpleSpider {
    pub(super) css: String,
    pub(super) start_urls: Vec<String>,
    pub(super) max_pages: usize,
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
        (items, vec![])
    }
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(MaxPages(self.max_pages))
    }
}
