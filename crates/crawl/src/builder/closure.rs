//! ClosureSpider — SpiderBuilder 构建出的闭包式 Spider 实现。

use super::Handler;
use crate::stop::StopCondition;
use crate::{BLOCKED_STATUS_CODES, Request, Response, Spider};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub(crate) type BlockedFn = Box<dyn Fn(&Response) -> bool + Send + Sync + 'static>;

/// 由 SpiderBuilder 构建的闭包式 Spider。
///
/// ND-031-ARCH：引擎配置字段已移除，ClosureSpider 只持有业务逻辑字段。
pub struct ClosureSpider {
    pub(crate) name: String,
    pub(crate) start_urls: Vec<String>,
    pub(crate) handlers: HashMap<String, Handler>,
    pub(crate) allowed_domains: HashSet<String>,
    pub(crate) is_blocked_fn: Option<BlockedFn>,
    pub(crate) until_cond: Arc<dyn StopCondition>,
    pub(crate) max_depth: u32,
}

#[async_trait]
impl Spider for ClosureSpider {
    fn name(&self) -> &str {
        &self.name
    }
    fn start_urls(&self) -> Vec<String> {
        self.start_urls.clone()
    }
    fn allowed_domains(&self) -> HashSet<String> {
        self.allowed_domains.clone()
    }

    fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// callback 路由：根据 `resp.request.callback` 查表分发。
    ///
    /// 路由顺序：
    /// 1. callback 为 `None` 或 `"default"` → "default" handler（若有）
    /// 2. callback 为其他 label → 对应 handler（若有）
    /// 3. label 无匹配 → 回退到 "default" handler
    /// 4. 都无 → 返回空
    async fn handle(&self, resp: Response) -> (Vec<Value>, Vec<Request>) {
        let label = resp.request.callback.as_deref().unwrap_or("default");
        match self.handlers.get(label) {
            Some(h) => h(resp).await,
            None => {
                // label 不匹配，回退到 "default" handler
                if let Some(default_h) = self.handlers.get("default") {
                    default_h(resp).await
                } else {
                    // 无 default handler，返回空
                    (vec![], vec![])
                }
            }
        }
    }

    fn is_blocked(&self, resp: &Response) -> bool {
        if let Some(ref f) = self.is_blocked_fn {
            f(resp)
        } else {
            BLOCKED_STATUS_CODES.contains(&resp.status)
        }
    }

    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::clone(&self.until_cond)
    }

    fn accepts_callback(&self, callback: Option<&str>) -> bool {
        match callback {
            None => self.handlers.contains_key("default"),
            Some(cb) => self.handlers.contains_key(cb),
        }
    }
}
