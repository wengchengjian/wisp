use super::*;
use async_trait::async_trait;
use wisp_browser::Page;
use wisp_core::error::Result;
use wisp_core::{Request, Response};

/// MockStrategy：用于验证 trait 可实现、可调用。
struct MockStrategy;

#[async_trait]
impl BrowserFetchStrategy for MockStrategy {
    async fn fetch(&self, _page: &mut Page, req: &Request) -> Result<Response> {
        Ok(Response::from_browser(
            200,
            req.url.clone(),
            "<html></html>".to_string(),
            "mock".to_string(),
            Vec::new(),
            req.clone(),
        ))
    }
}

#[test]
fn test_trait_object_can_be_constructed() {
    let strategy: Box<dyn BrowserFetchStrategy> = Box::new(MockStrategy);
    // 仅验证 trait object 可构造（无 UB）
    let _ = &*strategy as *const dyn BrowserFetchStrategy;
}
