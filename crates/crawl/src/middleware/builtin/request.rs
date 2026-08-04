//! Builtin 中间件子模块：request。

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

use super::{CrawlContext, ErrorAction, Middleware, RequestMwAction};
use crate::CrawlRequest;
use wisp_core::error::WispError;

// === 请求修改类 ===

/// UA 轮换中间件：每次请求随机选择一个 User-Agent。
pub struct UaRotationMiddleware {
    agents: Vec<String>,
    index: std::sync::atomic::AtomicUsize,
}

impl UaRotationMiddleware {
    /// 使用桌面 UA 列表创建（Chrome/Edge 136，匹配默认 TLS 指纹）。
    pub fn desktop() -> Self {
        Self {
            agents: vec![
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".into(),
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".into(),
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".into(),
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0".into(),
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0".into(),
            ],
            index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// 使用自定义 UA 列表创建。
    pub fn with_agents(agents: Vec<String>) -> Self {
        Self {
            agents,
            index: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Middleware for UaRotationMiddleware {
    fn priority(&self) -> u32 {
        20
    }

    async fn process_request(
        &self,
        req: &mut CrawlRequest,
        _ctx: &CrawlContext,
    ) -> RequestMwAction {
        if self.agents.is_empty() {
            return RequestMwAction::Continue;
        }
        let idx = self
            .index
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.agents.len();
        req.headers
            .insert("User-Agent".to_string(), self.agents[idx].clone());
        RequestMwAction::Modified
    }
}

/// 重试中间件：决定网络错误是否值得重试。
///
/// 职责单一（修复 ND-002-CORR）：
/// - **只决定**：这个错误是否值得重试（业务决策）
/// - **不维护**：重试计数和上限由 engine 在 `fetch_with_retry` 内统一管理
///
/// engine 读取 `EngineConfig.max_retries` 作为上限，维护 `req.retry_count` 计数。
/// 中间件只返回 `ErrorAction::Retry` 或 `Propagate`，不再读取/写入 `meta["_retry"]`。
pub struct RetryMiddleware {
    retry_delay: Duration,
}

impl RetryMiddleware {
    /// 创建重试中间件。
    ///
    /// - `retry_delay`：重试前的固定退避延迟（在中间件内 sleep）
    pub fn new(retry_delay: Duration) -> Self {
        Self { retry_delay }
    }
}

#[async_trait]
impl Middleware for RetryMiddleware {
    fn priority(&self) -> u32 {
        90
    }

    async fn process_error(
        &self,
        _req: &CrawlRequest,
        _err: &WispError,
        _ctx: &CrawlContext,
    ) -> ErrorAction {
        // fetch_page 返回 Err 都是网络层错误（DNS/连接/TLS/超时等），
        // HTTP 业务错误（4xx/5xx）会返回 Ok(resp)，由 BlockedRetryMiddleware 通过 Refetch 处理。
        // 因此这里默认重试所有 fetch 错误，计数和上限由 engine 在 fetch_with_retry 内统一管理。
        if !self.retry_delay.is_zero() {
            tokio::time::sleep(self.retry_delay).await;
        }
        ErrorAction::Retry
    }
}

/// 代理注入中间件：从代理池中为每个请求分配代理。
///
/// 代理由中间件全权管理，引擎仅读取 `req.proxy` 并应用。
pub struct ProxyInjectionMiddleware {
    pool: Arc<wisp_proxy::ProxyPool>,
}

impl ProxyInjectionMiddleware {
    /// 创建代理注入中间件。
    pub fn new(pool: Arc<wisp_proxy::ProxyPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Middleware for ProxyInjectionMiddleware {
    fn priority(&self) -> u32 {
        30
    }

    async fn process_request(
        &self,
        req: &mut CrawlRequest,
        _ctx: &CrawlContext,
    ) -> RequestMwAction {
        if let Some(proxy) = self.pool.next() {
            req.proxy = Some(proxy);
            RequestMwAction::Modified
        } else {
            RequestMwAction::Continue
        }
    }
}

/// 请求头注入中间件：为每个请求添加固定 headers。
pub struct HeadersMiddleware {
    headers: Vec<(String, String)>,
}

impl HeadersMiddleware {
    /// 创建请求头注入中间件。
    pub fn new(headers: Vec<(String, String)>) -> Self {
        Self { headers }
    }
}

#[async_trait]
impl Middleware for HeadersMiddleware {
    fn priority(&self) -> u32 {
        10
    }

    async fn process_request(
        &self,
        req: &mut CrawlRequest,
        _ctx: &CrawlContext,
    ) -> RequestMwAction {
        if self.headers.is_empty() {
            return RequestMwAction::Continue;
        }
        for (k, v) in &self.headers {
            req.headers.insert(k.clone(), v.clone());
        }
        RequestMwAction::Modified
    }
}
