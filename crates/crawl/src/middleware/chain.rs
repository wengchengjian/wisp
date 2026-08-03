//! 中间件链执行器。

use std::sync::Arc;

use serde_json::Value;
use tracing::Instrument;

use super::{
    CrawlContext, ErrorAction, ItemPipeline, Middleware, RequestMwAction, ResponseMwAction,
};
use crate::{Item, Request, Response};
use wisp_core::error::WispError;

/// 中间件链：按 priority 排序后顺序执行所有中间件。
pub(crate) struct MiddlewareChain {
    pub middlewares: Vec<Arc<dyn Middleware>>,
    pub pipelines: Vec<Arc<dyn ItemPipeline>>,
}

impl MiddlewareChain {
    pub fn new() -> Self {
        Self {
            middlewares: Vec::new(),
            pipelines: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.middlewares.is_empty() && self.pipelines.is_empty()
    }

    /// 中间件名称列表（按执行顺序）。
    pub fn names(&self) -> Vec<&'static str> {
        self.middlewares.iter().map(|mw| mw.name()).collect()
    }

    /// 按 priority 排序中间件（越小越先执行）。
    pub(crate) fn sort(&mut self) {
        self.middlewares.sort_by_key(|mw| mw.priority());
    }

    /// 初始化所有中间件（Engine 在爬取开始前调用）。
    pub(crate) async fn run_init(&self, ctx: &CrawlContext) {
        tracing::debug!("middleware chain: {}", self.names().join(" -> "));
        for mw in &self.middlewares {
            mw.init(ctx).await;
        }
    }

    /// 执行请求中间件链。返回 `RequestMwAction::Skip/Abort/Respond` 时中断。
    #[tracing::instrument(level = "trace", skip(self, req, ctx))]
    pub(crate) async fn run_request_middlewares(
        &self,
        req: &mut Request,
        ctx: &CrawlContext,
    ) -> RequestMwAction {
        for mw in &self.middlewares {
            let span = tracing::trace_span!(target: "middleware", "request", mw = mw.name());
            match mw.process_request(req, ctx).instrument(span).await {
                RequestMwAction::Continue | RequestMwAction::Modified => continue,
                action => return action,
            }
        }
        RequestMwAction::Continue
    }

    /// 执行响应中间件链。返回 `ResponseMwAction::Skip/Abort/Refetch` 时中断。
    #[tracing::instrument(level = "trace", skip(self, resp, ctx))]
    pub(crate) async fn run_response_middlewares(
        &self,
        resp: &mut Response,
        ctx: &CrawlContext,
    ) -> ResponseMwAction {
        for mw in &self.middlewares {
            let span = tracing::trace_span!(target: "middleware", "response", mw = mw.name());
            match mw.process_response(resp, ctx).instrument(span).await {
                ResponseMwAction::Continue | ResponseMwAction::Modified => continue,
                action => return action,
            }
        }
        ResponseMwAction::Continue
    }

    /// 执行错误中间件链。任一返回 Retry 则重试。
    pub(crate) async fn run_error_middlewares(
        &self,
        req: &Request,
        err: &WispError,
        ctx: &CrawlContext,
    ) -> ErrorAction {
        for mw in &self.middlewares {
            match mw.process_error(req, err, ctx).await {
                ErrorAction::Propagate => continue,
                action => return action,
            }
        }
        ErrorAction::Propagate
    }

    /// 执行 item 管道链。
    pub(crate) async fn run_pipelines(
        &self,
        item: Item<Value>,
        ctx: &CrawlContext,
    ) -> Option<Item<Value>> {
        let mut current = Some(item);
        for pipeline in &self.pipelines {
            match current {
                Some(item) => {
                    current = pipeline.process_item(item, ctx).await;
                }
                None => return None,
            }
        }
        current
    }

    /// 打开所有 pipeline（爬取开始前调用）。
    pub(crate) async fn run_pipelines_open(&self, ctx: &CrawlContext) {
        for pipeline in &self.pipelines {
            pipeline.open(ctx).await;
        }
    }

    /// 关闭所有 pipeline（爬取结束后调用）。
    pub(crate) async fn run_pipelines_close(&self, ctx: &CrawlContext) {
        for pipeline in &self.pipelines {
            pipeline.close(ctx).await;
        }
    }
}
