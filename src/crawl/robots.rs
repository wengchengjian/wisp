//! robots.txt parsing and caching.

use std::collections::HashMap;
use crate::fetch::Client;

/// Cache of robots.txt rules per domain.
pub struct RobotsCache {
    cache: HashMap<String, Vec<String>>,  // domain -> disallowed paths
}

impl RobotsCache {
    pub fn new() -> Self { Self { cache: HashMap::new() } }

    /// Check if a URL is allowed by robots.txt.
    pub async fn is_allowed(&mut self, client: &Client, url: &str) -> bool {
        let Ok(parsed) = url::Url::parse(url) else { return true; };
        let Some(host) = parsed.host_str() else { return true; };
        let domain = format!("{}://{}", parsed.scheme(), host);

        if !self.cache.contains_key(&domain) {
            let rules = self.fetch_robots(client, &domain).await;
            self.cache.insert(domain.clone(), rules);
        }

        let disallowed = self.cache.get(&domain).unwrap();
        let path = parsed.path();
        !disallowed.iter().any(|d| path.starts_with(d))
    }

    async fn fetch_robots(&self, client: &Client, domain: &str) -> Vec<String> {
        let robots_url = format!("{}/robots.txt", domain);
        let Ok(resp) = client.get(&robots_url).await else { return Vec::new() };
        let Ok(text) = resp.text() else { return Vec::new() };

        let mut disallowed = Vec::new();
        let mut in_our_section = false;

        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("User-agent:") {
                let agent = line["User-agent:".len()..].trim();
                in_our_section = agent == "*";
            } else if in_our_section && line.starts_with("Disallow:") {
                let path = line["Disallow:".len()..].trim();
                if !path.is_empty() {
                    disallowed.push(path.to_string());
                }
            }
        }
        disallowed
    }
}
