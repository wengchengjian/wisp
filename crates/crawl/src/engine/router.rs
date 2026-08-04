//! RequestRouter — 跨 Spider 请求路由的独立深模块。
//!
//! 决定一个 `CrawlRequest` 应路由到哪个 Spider，策略优先级：
//! `Request.spider` 显式绑定 > callback 唯一归属 > 歧义（warn + None）。
//!
//! 从 `SpiderRegistry` 中提取，使路由决策可脱离 Engine 独立测试：
//! 显式命中 / 唯一 callback / 歧义 / 无归属四个分支均可单测。

use super::*;

/// 请求路由：持有 Spider 列表，暴露 `route()` 决策。
///
/// `spiders[i]` 与 Runner 侧 `all_stats[i]` 一一对应（索引由调用方保持一致）。
pub(crate) struct RequestRouter {
    pub(crate) spiders: Vec<Arc<dyn Spider>>,
}

impl RequestRouter {
    /// 创建路由，持有待路由的 Spider 列表。
    pub(crate) fn new(spiders: Vec<Arc<dyn Spider>>) -> Self {
        Self { spiders }
    }

    /// 返回请求应路由到的 Spider 索引；`Request.spider` 显式指定优先，
    /// 否则按 callback 唯一归属查找，多个 Spider 接受同一 callback 视为歧义（None）。
    pub(crate) fn route(&self, req: &CrawlRequest) -> Option<usize> {
        if let Some(name) = req.spider.as_deref() {
            // 显式绑定优先：按名字精确匹配。
            let idx = self.spiders.iter().position(|s| s.name() == name);
            if let Some(idx) = idx {
                tracing::debug!(spider = name, index = idx, url = %sanitize_url(&req.url), "路由：显式绑定 spider");
            } else {
                tracing::warn!(spider = name, url = %sanitize_url(&req.url), "路由：显式绑定到不存在的 spider");
            }
            return idx;
        }
        // callback 是跨 Spider 的路由键（小说爬虫 home→detail→chapter）；
        // 只有唯一 Spider 接受时才按 callback 路由，多个 Spider 同名 handler 视为歧义。
        let matches: Vec<usize> = self
            .spiders
            .iter()
            .enumerate()
            .filter(|(_, s)| s.accepts_callback(req.callback.as_deref()))
            .map(|(i, _)| i)
            .collect();
        match matches.as_slice() {
            [idx] => {
                tracing::debug!(callback = ?req.callback, index = *idx, url = %sanitize_url(&req.url), "路由：唯一 callback 归属");
                Some(*idx)
            }
            [] => {
                tracing::warn!(callback = ?req.callback, url = %sanitize_url(&req.url), "路由：无 Spider 接受该 callback");
                None
            }
            _ => {
                tracing::warn!(
                    callback = ?req.callback,
                    matches = matches.len(),
                    url = %sanitize_url(&req.url),
                    "路由：callback 被多个 Spider 接受，未绑定 spider 的请求无法路由"
                );
                None
            }
        }
    }

    /// 只读访问 Spider 列表（同时用于 stats 索引保持一致的遍历）。
    pub(crate) fn spiders(&self) -> &[Arc<dyn Spider>] {
        &self.spiders
    }

    /// 按索引取 Spider；越界返回 `None`。
    pub(crate) fn get(&self, idx: usize) -> Option<&Arc<dyn Spider>> {
        self.spiders.get(idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Spider;
    use serde_json::Value;
    use std::collections::HashSet;
    use wisp_fetcher::Response;

    /// Mock Spider：`name` 控制显式路由，`callbacks` 控制 `accepts_callback`。
    struct MockSpider {
        name: &'static str,
        callbacks: HashSet<String>,
    }

    #[async_trait::async_trait]
    impl Spider for MockSpider {
        fn name(&self) -> &str {
            self.name
        }
        fn start_urls(&self) -> Vec<String> {
            vec![]
        }
        async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<CrawlRequest>) {
            (vec![], vec![])
        }
        fn accepts_callback(&self, callback: Option<&str>) -> bool {
            callback.map_or(true, |c| self.callbacks.contains(c))
        }
    }

    fn spider(name: &'static str, callbacks: &[&str]) -> Arc<dyn Spider> {
        Arc::new(MockSpider {
            name,
            callbacks: callbacks.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn router(spiders: Vec<Arc<dyn Spider>>) -> RequestRouter {
        RequestRouter::new(spiders)
    }

    #[test]
    fn route_prefers_explicit_spider_binding() {
        let r = router(vec![
            spider("list", &["default"]),
            spider("detail", &["detail"]),
        ]);
        let req = CrawlRequest::get("http://example.com/detail").with_spider("detail");
        assert_eq!(
            r.route(&req),
            Some(1),
            "显式 spider 绑定应覆盖 callback 归属"
        );
    }

    #[test]
    fn route_uses_unique_callback_owner() {
        let r = router(vec![
            spider("list", &["default"]),
            spider("detail", &["detail"]),
        ]);
        let req = CrawlRequest::get("http://example.com/detail").with_callback("detail");
        assert_eq!(
            r.route(&req),
            Some(1),
            "唯一接受 callback 的 Spider 应被选中"
        );
    }

    #[test]
    fn route_ambiguous_callback_returns_none() {
        // 两个 Spider 都接受 "detail"，未绑定 spider 时视为歧义。
        let r = router(vec![spider("a", &["detail"]), spider("b", &["detail"])]);
        let req = CrawlRequest::get("http://example.com/detail").with_callback("detail");
        assert_eq!(r.route(&req), None, "同名 callback 多 Spider 应视为歧义");
    }

    #[test]
    fn route_no_match_returns_none() {
        let r = router(vec![spider("list", &["default"])]);
        let req = CrawlRequest::get("http://example.com/detail").with_callback("detail");
        assert_eq!(r.route(&req), None, "无 Spider 接受 callback 应返回 None");
    }

    #[test]
    fn route_explicit_binding_to_unknown_spider_returns_none() {
        let r = router(vec![spider("list", &["default"])]);
        let req = CrawlRequest::get("http://example.com/").with_spider("ghost");
        assert_eq!(r.route(&req), None, "显式绑定到不存在的 Spider 应返回 None");
    }
}
