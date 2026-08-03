//! EngineBuilder 配置方法。

use super::*;
use std::sync::Arc;
use std::time::Duration;
use wisp_fetcher::{FetchClient, FetchClientConfig, FetchMode};

impl EngineBuilder {
    /// 设置最大并发数。
    pub fn max_concurrent(mut self, n: usize) -> Self {
        self.config.max_concurrent = n;
        self
    }

    /// 使用已有 FetchClient（MCP 等需要共享 HTTP/BrowserPool 的场景）。
    pub fn fetch_client(mut self, client: Arc<FetchClient>) -> Self {
        self.fetch_client = Some(client);
        self
    }

    /// 设置最大爬取页数。
    pub fn max_pages(mut self, n: usize) -> Self {
        self.config.max_pages = n;
        self
    }

    /// 设置 FetchClient 配置（HTTP 连接池/超时/浏览器等基础设施配置，跨 Spider 共享）。
    pub fn fetch_client_config(mut self, config: FetchClientConfig) -> Self {
        self.config.transport = config;
        self
    }

    /// 设置代理（作用于共享 FetchClient 的所有 HTTP 请求）。
    pub fn proxy(mut self, proxy: &str) -> Self {
        self.config.transport.proxy = Some(proxy.to_string());
        self
    }

    /// 设置中间件 Refetch 最大轮数（默认 5）。
    pub fn max_refetch_rounds(mut self, n: usize) -> Self {
        self.config.max_refetch_rounds = n;
        self
    }

    /// 启用响应缓存（注入 CacheMiddleware，默认 TTL 5 分钟）。
    pub fn cache_store(mut self, store: Arc<dyn wisp_storage::Store>) -> Self {
        self.cache_store = Some(store);
        self.config.cache_enabled = true;
        self
    }

    /// 设置检查点存储（定期保存爬取进度）。
    pub fn checkpoint(mut self, s: Arc<dyn wisp_storage::Store>, interval: usize) -> Self {
        self.checkpoint_store = Some(s);
        self.config.checkpoint_interval = interval;
        self.config.checkpoint_enabled = true;
        self
    }

    /// 启用自适应并发池。min 为初始/下限，max 为上限。
    pub fn autoscale(mut self, min: usize, max: usize) -> Self {
        self.autoscale = Some(crate::runtime::autoscale::AutoscaledPool::new(
            min,
            max,
            crate::runtime::autoscale::AutoscaleConfig::default(),
        ));
        self
    }

    /// 同 autoscale(min, max) 但可自定义配置。
    pub fn autoscale_with_config(
        mut self,
        min: usize,
        max: usize,
        config: crate::runtime::autoscale::AutoscaleConfig,
    ) -> Self {
        self.autoscale = Some(crate::runtime::autoscale::AutoscaledPool::new(
            min, max, config,
        ));
        self
    }

    /// 设置抓取模式（Http/Dynamic/Stealth/Auto，默认 Auto）。
    pub fn fetch_mode(mut self, mode: FetchMode) -> Self {
        self.config.fetch_mode = mode;
        self
    }

    /// 是否遵守 robots.txt（默认 true）。
    pub fn obey_robots(mut self, obey: bool) -> Self {
        self.config.obey_robots = obey;
        self
    }

    /// 设置网络错误重试上限（默认 3）。
    pub fn max_retries(mut self, n: u32) -> Self {
        self.config.max_retries = n;
        self
    }

    /// 设置下载延迟（默认 0，即无延迟）。
    pub fn download_delay(mut self, d: Duration) -> Self {
        self.config.download_delay = d;
        self
    }

    /// 设置下载延迟（毫秒）。
    pub fn download_delay_ms(mut self, ms: u64) -> Self {
        self.config.download_delay = Duration::from_millis(ms);
        self
    }

    /// 设置固定请求头（Engine 级传输能力）。
    pub fn headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.config.headers = headers;
        self
    }

    /// 设置 UA 轮换策略；不调用则请求不带 User-Agent。
    pub fn ua_rotation(mut self, ua: crate::middleware::UaRotationMiddleware) -> Self {
        self.ua_middleware = Some(Arc::new(ua));
        self.config.ua_rotation = true;
        self
    }

    /// 是否启用 Cookie Challenge 自动处理。
    pub fn cookie_challenge(mut self, enabled: bool) -> Self {
        self.config.cookie_challenge = enabled;
        self
    }

    /// Auto 模式是否启用 SPA/DOM 动态升级扫描。
    pub fn dynamic_upgrade(mut self, enabled: bool) -> Self {
        self.config.dynamic_upgrade = enabled;
        self
    }

    /// 添加自定义 Engine 级中间件。
    pub fn middleware(mut self, mw: Arc<dyn crate::middleware::Middleware>) -> Self {
        self.custom_middlewares.push(mw);
        self
    }

    /// 添加共享 item pipeline（所有 Spider 的 item 汇入同一链）。
    pub fn pipeline(mut self, p: Arc<dyn crate::middleware::ItemPipeline>) -> Self {
        self.pipelines.push(p);
        self
    }

    /// Auto 模式：添加 URL 正则规则（优先级最高，跳过嗅探）。
    pub fn auto_rule(mut self, pattern: &str, mode: FetchMode) -> Self {
        self.config.auto_rules.push((pattern.to_string(), mode));
        self
    }

    /// 设置引擎内部事件总线。
    pub fn event_bus(mut self, bus: EventBus) -> Self {
        self.event_bus = bus;
        self
    }

    /// 注册一个引擎事件监听器（等价于 `event_bus` + `bus.on(listener)`）。
    pub fn event_listener(mut self, listener: impl EventCallback + Send + Sync + 'static) -> Self {
        self.event_bus.on(listener);
        self
    }
}
