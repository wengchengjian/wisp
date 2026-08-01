//! MCP 工具实现。

mod adaptive;
mod crawl;
mod extract;
mod fetch;
mod spider;
mod stealth;

pub use adaptive::adaptive_scrape;
pub use crawl::crawl_site;
pub use extract::extract_css;
pub use fetch::fetch_page;
#[cfg(feature = "browser")]
pub use stealth::stealth_fetch;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use wisp_storage::Store;

    #[tokio::test]
    async fn test_extract_css_returns_text() {
        let args = json!({
            "html": "<html><body><p class='x'>hello</p><p class='x'>world</p></body></html>",
            "selector": "p.x"
        });
        let result = extract_css(args).await.unwrap();
        let texts = result["texts"].as_array().unwrap();
        assert_eq!(texts.len(), 2);
        assert_eq!(texts[0].as_str().unwrap(), "hello");
        assert_eq!(texts[1].as_str().unwrap(), "world");
    }

    #[tokio::test]
    async fn test_extract_css_returns_attr() {
        let args = json!({
            "html": "<html><body><a href='/a'>A</a><a href='/b'>B</a></body></html>",
            "selector": "a",
            "attr": "href"
        });
        let result = extract_css(args).await.unwrap();
        let attrs = result["attrs"].as_array().unwrap();
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].as_str().unwrap(), "/a");
    }

    #[tokio::test]
    async fn test_extract_css_missing_args() {
        let args = json!({});
        let result = extract_css(args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_page_missing_url() {
        let args = json!({});
        let result = fetch_page(args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_crawl_site_missing_args() {
        let engine = wisp_crawl::Engine::infra().max_pages(100).build().unwrap();
        let args = json!({});
        let result = crawl_site(args, &engine).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_adaptive_scrape_missing_args() {
        let store: Arc<dyn Store> = Arc::new(wisp_storage::MemoryStore::default());
        let args = json!({});
        let result = adaptive_scrape(args, &store).await;
        assert!(result.is_err());
    }

    #[cfg(feature = "browser")]
    #[tokio::test]
    async fn test_stealth_fetch_missing_url() {
        let args = json!({});
        let result = stealth_fetch(args).await;
        assert!(result.is_err());
    }
}
