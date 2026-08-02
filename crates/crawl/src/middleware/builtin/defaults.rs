//! Builtin 中间件子模块：defaults。

use super::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use super::Middleware;
use crate::auto::ModeRuleEngine;
use crate::runtime::robots::RobotsCache;
use wisp_fetcher::FetchClient;
use wisp_fetcher::FetchMode;
use wisp_storage::Store;

// === 默认中间件注入 ===

/// 默认中间件注入配置（由 Spider 配置 + Engine 资源组装）。
///
/// `default_middlewares` 据此构造中间件链；字段对应各默认中间件所需的输入。
pub struct DefaultMiddlewareConfig {
    /// 抓取模式（决定是否注入模式升级类）
    pub fetch_mode: FetchMode,
    /// 下载延迟（>0 时注入 DelayMiddleware）
    pub delay: Duration,
    /// 固定请求头（非空时注入 HeadersMiddleware）
    pub headers: Vec<(String, String)>,
    /// UA 轮换策略（Some 时注入 UaRotationMiddleware）
    pub ua_middleware: Option<Arc<UaRotationMiddleware>>,
    /// 是否启用 Cookie Challenge 自动处理
    pub cookie_challenge: bool,
    /// Auto 模式是否启用 DynamicUpgrade 扫描
    pub dynamic_upgrade: bool,
    /// 是否遵守 robots.txt（true 时注入 RobotsMiddleware）
    pub obey_robots: bool,
    /// 响应缓存存储（Some 时注入 CacheMiddleware，默认 TTL 5 分钟）
    pub cache_store: Option<Arc<dyn Store>>,
    /// HTTP 客户端（RobotsMiddleware 拉取 robots.txt 用）
    pub http_client: Arc<FetchClient>,
    /// robots 缓存（跨请求共享 robots 规则，内部 DashMap 无锁读）
    pub robots_cache: Arc<RobotsCache>,
    /// Auto 模式规则引擎（StealthUpgradeMiddleware 学习模式用）
    pub rule_engine: Arc<Mutex<ModeRuleEngine>>,
}

/// 按 FetchMode 和 Spider 配置注入默认行为中间件链。
///
/// 中间件分 4 类（按 priority 升序，由 `MiddlewareChain::sort` 统一排序）：
/// 1. **过滤类**（0-8）：Cache / Robots — 按配置启用
/// 2. **请求修改类**（10-30）：Delay / UaRotation — delay>0 / 总是
/// 3. **模式升级类**（40-45）：DynamicUpgrade（可选）/ StealthUpgrade — **仅 Auto 模式**
/// 4. **重试/挑战类**（50-90）：CookieChallenge / BlockedRetry / Retry — 总是
///
/// Http/Dynamic/Stealth 模式不注入升级类（用户已明确选择模式）；
/// Auto 模式注入 Stealth 升级；DynamicUpgrade 按 `dynamic_upgrade` 开关注入，
/// 避免静态站点为每页付出 SPA 扫描成本。
///
pub fn default_middlewares(cfg: DefaultMiddlewareConfig) -> Vec<Arc<dyn Middleware>> {
    let mut mws: Vec<Arc<dyn Middleware>> = Vec::new();

    // 1. 过滤类
    if let Some(store) = cfg.cache_store.clone() {
        // 默认 TTL 5 分钟：响应缓存有界，避免长爬取中缓存体无界占用内存。
        mws.push(Arc::new(CacheMiddleware::new(
            store,
            Some(Duration::from_secs(300)),
        )));
    }
    if cfg.obey_robots {
        mws.push(Arc::new(RobotsMiddleware::new(
            cfg.robots_cache,
            cfg.http_client,
        )));
    }

    // 2. 请求修改类
    if !cfg.delay.is_zero() {
        mws.push(Arc::new(DelayMiddleware::new(cfg.delay)));
    }
    if !cfg.headers.is_empty() {
        mws.push(Arc::new(HeadersMiddleware::new(cfg.headers)));
    }
    if let Some(ua) = cfg.ua_middleware {
        mws.push(ua);
    }

    // 3. 模式升级类（仅 Auto）
    if cfg.fetch_mode == FetchMode::Auto {
        if cfg.dynamic_upgrade {
            mws.push(Arc::new(DynamicUpgradeMiddleware::new()));
        }
        mws.push(Arc::new(StealthUpgradeMiddleware::new(cfg.rule_engine)));
    }

    // 4. 重试/挑战类
    if cfg.cookie_challenge {
        mws.push(Arc::new(CookieChallengeMiddleware::default()));
    }
    mws.push(Arc::new(BlockedRetryMiddleware::default()));
    mws.push(Arc::new(RetryMiddleware::new(Duration::from_millis(500))));

    mws
}
