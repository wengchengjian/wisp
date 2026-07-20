//! Spider templates: CrawlSpider and SitemapSpider.

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;

use super::{Spider, SpiderRequest, SpiderResponse};

/// A rule for CrawlSpider.
#[derive(Clone)]
pub struct CrawlRule {
    pub pattern: String,  // regex pattern for URLs to follow
    pub callback: Option<String>,
    pub follow: bool,
}

/// Rule-based crawling spider.
pub struct CrawlSpider {
    pub name: String,
    pub start_urls: Vec<String>,
    pub rules: Vec<CrawlRule>,
    pub allowed_domains: HashSet<String>,
}

#[async_trait]
impl Spider for CrawlSpider {
    fn name(&self) -> &str { &self.name }
    fn start_urls(&self) -> Vec<String> { self.start_urls.clone() }
    fn allowed_domains(&self) -> HashSet<String> { self.allowed_domains.clone() }

    async fn parse(&self, response: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let Ok(doc) = response.parse() else { return (vec![], vec![]); };
        let mut follow = Vec::new();

        // Extract all links and match against rules
        let links = doc.select("a[href]");
        for link in links.iter() {
            if let Some(href) = link.attr("href") {
                let absolute = resolve_url(&response.url, &href);
                for rule in &self.rules {
                    if let Ok(re) = Regex::new(&rule.pattern) {
                        if re.is_match(&absolute) {
                            let mut req = SpiderRequest::get(&absolute);
                            if let Some(cb) = &rule.callback {
                                req = req.with_callback(cb);
                            }
                            follow.push(req);
                            break;
                        }
                    }
                }
            }
        }
        (vec![], follow)
    }
}

/// Sitemap-based crawling spider.
pub struct SitemapSpider {
    pub name: String,
    pub sitemap_urls: Vec<String>,
    pub allowed_domains: HashSet<String>,
}

#[async_trait]
impl Spider for SitemapSpider {
    fn name(&self) -> &str { &self.name }
    fn start_urls(&self) -> Vec<String> { self.sitemap_urls.clone() }
    fn allowed_domains(&self) -> HashSet<String> { self.allowed_domains.clone() }

    async fn parse(&self, response: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let Ok(text) = response.text() else { return (vec![], vec![]); };
        let mut follow = Vec::new();

        // Parse sitemap XML: extract <loc> URLs
        let re = Regex::new(r"<loc>\s*(.*?)\s*</loc>").unwrap();
        for cap in re.captures_iter(&text) {
            if let Some(url) = cap.get(1) {
                let url = url.as_str().trim();
                if url.ends_with(".xml") {
                    // Nested sitemap
                    follow.push(SpiderRequest::get(url));
                } else {
                    follow.push(SpiderRequest::get(url));
                }
            }
        }
        (vec![], follow)
    }
}

fn resolve_url(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    if let Ok(base_url) = url::Url::parse(base) {
        if let Ok(resolved) = base_url.join(href) {
            return resolved.to_string();
        }
    }
    href.to_string()
}
