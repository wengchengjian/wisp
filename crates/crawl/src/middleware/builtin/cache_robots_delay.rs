//! Builtin 中间件子模块：limit。

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

use super::{CrawlContext, Middleware, RequestMwAction, ResponseMwAction};
use crate::runtime::robots::RobotsCache;
use crate::{Request, Response};
use wisp_fetcher::FetchClient;
use wisp_storage::{CachedResponse, Store};

// === 过滤/限制类 ===

/// 响应缓存中间件：缓存命中时通过 `RequestMwAction::Respond` 短路，跳过网络请求。
///
/// 响应返回后自动写入缓存。TTL 由 `default_ttl` 决定（写入 `CachedResponse.ttl`）。
pub struct CacheMiddleware {
    store: Arc<dyn Store>,
    default_ttl: Option<Duration>,
}

impl CacheMiddleware {
    /// 构造缓存中间件。`default_ttl` 为 `None` 时缓存永不过期。
    pub fn new(store: Arc<dyn Store>, default_ttl: Option<Duration>) -> Self {
        Self { store, default_ttl }
    }
}

#[async_trait]
impl Middleware for CacheMiddleware {
    fn priority(&self) -> u32 {
        3
    }

    async fn process_request(&self, req: &mut Request, _ctx: &CrawlContext) -> RequestMwAction {
        let method_str = req.method.as_str();
        match wisp_storage::load_response(&*self.store, method_str, &req.url).await {
            Ok(Some(cached)) => {
                let resp = Response::from_parts(wisp_core::ResponseParts {
                    status: cached.status,
                    url: req.url.clone(),
                    headers: cached.headers,
                    body: cached.body,
                    title: None,
                    cookies: Vec::new(),
                    request: req.clone(),
                    content_type: cached.content_type,
                    from_cache: true,
                });
                return RequestMwAction::Respond(resp);
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("缓存读取失败: {}", e),
        }
        RequestMwAction::Continue
    }

    async fn process_response(&self, resp: &mut Response, _ctx: &CrawlContext) -> ResponseMwAction {
        if resp.status >= 200 && resp.status < 400 && !resp.from_cache {
            let method_str = resp.request.method.as_str();
            let cached = CachedResponse {
                status: resp.status,
                headers: resp.headers.clone(),
                body: resp.body.clone(),
                content_type: resp.content_type.clone(),
                cached_at: chrono::Utc::now().timestamp(),
                ttl: self.default_ttl,
            };
            if let Err(e) =
                wisp_storage::save_response(&*self.store, method_str, &resp.url, &cached).await
            {
                tracing::warn!("响应缓存写入失败: {}", e);
            }
        }
        ResponseMwAction::Continue
    }
}

/// Robots.txt 检查中间件：请求前检查目标 URL 是否被 robots.txt 禁止。
///
/// `RobotsCache` 内部用 `DashMap` 实现无锁读 + fetch 时不持锁，
/// 因此 `RobotsMiddleware` 无需额外 `Mutex` 包裹，多个并发请求可并行检查。
pub struct RobotsMiddleware {
    robots_cache: Arc<RobotsCache>,
    fetch_client: Arc<FetchClient>,
}

impl RobotsMiddleware {
    /// 创建 Robots.txt 检查中间件。
    pub fn new(robots_cache: Arc<RobotsCache>, fetch_client: Arc<FetchClient>) -> Self {
        Self {
            robots_cache,
            fetch_client,
        }
    }
}

#[async_trait]
impl Middleware for RobotsMiddleware {
    fn priority(&self) -> u32 {
        8
    }

    async fn process_request(&self, req: &mut Request, _ctx: &CrawlContext) -> RequestMwAction {
        let allowed = self
            .robots_cache
            .is_allowed(&self.fetch_client, &req.url)
            .await;
        if allowed {
            RequestMwAction::Continue
        } else {
            RequestMwAction::Skip
        }
    }
}

/// 下载延迟中间件：每个请求发出前等待固定时间，避免过快访问。
pub struct DelayMiddleware {
    delay: Duration,
}

impl DelayMiddleware {
    /// 创建下载延迟中间件。
    pub fn new(delay: Duration) -> Self {
        Self { delay }
    }

    /// 便捷构造：毫秒数。
    pub fn from_millis(ms: u64) -> Self {
        Self {
            delay: Duration::from_millis(ms),
        }
    }
}

#[async_trait]
impl Middleware for DelayMiddleware {
    fn priority(&self) -> u32 {
        15
    }

    async fn process_request(&self, _req: &mut Request, _ctx: &CrawlContext) -> RequestMwAction {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        RequestMwAction::Continue
    }
}
