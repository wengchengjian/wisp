//! Builtin 中间件子模块：upgrade。

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::{CrawlContext, Middleware, ResponseMwAction};
use crate::Response;
use crate::auto::{self, ModeRuleEngine};
use wisp_fetcher::FetchMode;

// === 模式升级类 ===

/// Stealth 升级中间件：HTTP 被拦截时自动升级为 Stealth 浏览器模式重取。
/// Content-Type 是否可能承载 HTML 挑战页；空值保守返回 true。
pub(crate) fn content_type_may_be_html(content_type: &str) -> bool {
    content_type.is_empty() || content_type.to_ascii_lowercase().contains("html")
}

pub struct StealthUpgradeMiddleware {
    rule_engine: Arc<Mutex<ModeRuleEngine>>,
}

impl StealthUpgradeMiddleware {
    /// 创建 Stealth 升级中间件。
    pub fn new(rule_engine: Arc<Mutex<ModeRuleEngine>>) -> Self {
        Self { rule_engine }
    }
}

#[async_trait]
impl Middleware for StealthUpgradeMiddleware {
    fn priority(&self) -> u32 {
        45
    }

    async fn process_response(&self, resp: &mut Response, _ctx: &CrawlContext) -> ResponseMwAction {
        // 已按明确模式抓取的响应（Dynamic/Stealth/HTTP+cookie 快速路径）不再需要升级。
        if resp.request.fetch_mode_override.is_some() {
            return ResponseMwAction::Continue;
        }
        // 非 HTML 响应不可能是 CF HTML 挑战页，跳过 body 扫描。
        if !content_type_may_be_html(&resp.content_type) {
            return ResponseMwAction::Continue;
        }
        if auto::is_blocked_response(resp.status, &resp.body, &resp.headers) {
            self.rule_engine
                .lock()
                .await
                .learn(&resp.url, FetchMode::Stealth);
            tracing::info!(
                "StealthUpgrade: '{}' 被拦截 (status={})，升级 Stealth",
                resp.url,
                resp.status
            );
            let mut new_req = resp.request.clone();
            new_req.fetch_mode_override = Some(FetchMode::Stealth);
            return ResponseMwAction::Refetch(new_req);
        }
        ResponseMwAction::Continue
    }
}

// === Dynamic 升级类 ===

/// SPA 框架标识：命中任一即为强信号（10 分），立即升级。
const SPA_FRAMEWORK_MARKERS: &[&str] = &[
    "__NUXT_DATA__",
    "__NEXT_DATA__",
    "react-app.embeddedData",
    "data-reactroot",
    "ng-version",
    "data-v-app",
    "gatsby-chunk-mapping",
    "/_nuxt/",
    "/_next/static/",
];

/// DOM 修改方法：命中任一即为中信号（7 分）。
const DOM_MUTATION_METHODS: &[&str] = &[
    ".createElement(",
    ".innerHTML",
    ".outerHTML",
    "history.pushState",
    "history.replaceState",
    "fetch(",
    "new XMLHttpRequest",
];

/// 弱信号阈值：外部脚本密度 >= 此值时触发（7 分）。
/// 借鉴 spider 框架的 `script_src_count >= 4`，但 wisp 统计所有 `<script` 标签
/// （无法流式提取 src 属性），因此阈值调高为 6。
const SCRIPT_DENSITY_THRESHOLD: usize = 6;
/// SPA 标记/脚本密度检测只需要页面头部；超大正文不参与扫描，避免每页全量匹配。
const MAX_DYNAMIC_SCAN_BYTES: usize = 256 * 1024;

/// Dynamic 升级中间件：检测页面可能需要 JS 渲染时升级到 Dynamic 模式。
///
/// 评分信号借鉴 spider 框架的 smart 模式：
/// - 强信号（10 分）：SPA 框架标识（`__NUXT_DATA__`、`__NEXT_DATA__` 等）
/// - 中信号（7 分）：DOM 修改方法（`.createElement(`、`.innerHTML`、`fetch(` 等）
/// - 弱信号（7 分）：`<script` 标签密度 >= 6
///
/// 评分 >= 7 时触发 `Refetch` + `fetch_mode_override = Dynamic`。
pub struct DynamicUpgradeMiddleware {
    spa_matcher: aho_corasick::AhoCorasick,
    dom_matcher: aho_corasick::AhoCorasick,
    script_matcher: aho_corasick::AhoCorasick,
}

impl Default for DynamicUpgradeMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl DynamicUpgradeMiddleware {
    /// 创建 Dynamic 升级中间件。
    pub fn new() -> Self {
        Self {
            spa_matcher: aho_corasick::AhoCorasick::new(SPA_FRAMEWORK_MARKERS)
                .expect("SPA markers should be valid"),
            dom_matcher: aho_corasick::AhoCorasick::new(DOM_MUTATION_METHODS)
                .expect("DOM mutation methods should be valid"),
            script_matcher: aho_corasick::AhoCorasick::new(["<script"])
                .expect("script pattern should be valid"),
        }
    }

    /// 评估响应 body 的 JS 渲染需求分数。
    fn score_body(&self, body: &[u8]) -> u8 {
        let body = &body[..body.len().min(MAX_DYNAMIC_SCAN_BYTES)];
        // 强信号：SPA 框架标识 → 直接满分
        if self.spa_matcher.find(body).is_some() {
            return 10;
        }
        // 中信号：DOM 修改方法 → 7 分
        if self.dom_matcher.find(body).is_some() {
            return 7;
        }
        // 弱信号：`<script` 标签密度 >= 6 → 7 分
        if self.script_matcher.find_iter(body).count() >= SCRIPT_DENSITY_THRESHOLD {
            return 7;
        }
        0
    }
}

#[async_trait]
impl Middleware for DynamicUpgradeMiddleware {
    fn priority(&self) -> u32 {
        40
    }

    async fn process_response(&self, resp: &mut Response, _ctx: &CrawlContext) -> ResponseMwAction {
        // 已有 override 不重复升级
        if resp.request.fetch_mode_override.is_some() {
            return ResponseMwAction::Continue;
        }
        // 仅对 200 响应检测
        if resp.status != 200 {
            return ResponseMwAction::Continue;
        }
        if self.score_body(&resp.body) >= 7 {
            let mut new_req = resp.request.clone();
            new_req.fetch_mode_override = Some(FetchMode::Dynamic);
            tracing::info!(
                "DynamicUpgrade: '{}' 检测到 SPA/DOM 动态特征，升级 Dynamic",
                resp.url
            );
            return ResponseMwAction::Refetch(new_req);
        }
        ResponseMwAction::Continue
    }
}
