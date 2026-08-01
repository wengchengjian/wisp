//! 统一请求类型（Fetcher + Spider 共用）。

use super::{FetchMode, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// 自定义 serde：把 `serde_json::Value` 编码为 `Vec<u8>` JSON 字节，
/// 绕过 bincode 1.x 不支持 `deserialize_any` 的限制，使 meta 随 checkpoint 往返。
mod meta_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer>(v: &Value, s: S) -> Result<S::Ok, S::Error> {
        let bytes = serde_json::to_vec(v).map_err(serde::ser::Error::custom)?;
        bytes.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
        let bytes = Vec::<u8>::deserialize(d)?;
        serde_json::from_slice(&bytes).map_err(serde::de::Error::custom)
    }
}

/// 统一请求类型（Fetcher + Spider 共用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// 请求 URL。
    pub url: String,
    /// HTTP 方法。
    pub method: Method,
    /// 请求头。
    pub headers: HashMap<String, String>,
    /// 请求体（POST/PUT 用）。
    pub body: Option<String>,
    /// 用户自定义元数据（Spider 场景传递深度、回调等）
    #[serde(with = "meta_serde")]
    pub meta: Value,
    /// Spider 回调名称
    pub callback: Option<String>,
    /// 多 Spider 模式下所属 Spider 名称（稳定 id）；None 表示尚未绑定。
    #[serde(default)]
    pub spider: Option<String>,
    /// 优先级（Spider 调度用）
    pub priority: i32,
    /// 深度：起始 URL 为 0，每 follow 一次 +1。
    #[serde(default)]
    pub depth: u32,
    /// 代理 URL（由 ProxyInjectionMiddleware 设置，引擎读取并应用）。
    #[serde(skip)]
    pub proxy: Option<String>,
    /// 抓取模式覆盖（由 StealthUpgradeMiddleware 等设置，引擎优先使用此模式）。
    #[serde(skip)]
    pub fetch_mode_override: Option<FetchMode>,
    /// 网络错误重试计数（由 engine 维护，中间件只读）。
    ///
    /// 与 `refetch_depth`（响应中间件 Refetch 计数，engine 局部变量）独立：
    /// - `retry_count`：fetch 失败后同步重试的次数，上限 `EngineConfig.max_retries`
    /// - `refetch_depth`：响应成功后业务重做的次数，上限 `EngineConfig.max_refetch_rounds`
    #[serde(skip)]
    pub retry_count: u32,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: Method::Get,
            headers: HashMap::new(),
            body: None,
            meta: Value::Null,
            callback: None,
            spider: None,
            priority: 0,
            depth: 0,
            proxy: None,
            fetch_mode_override: None,
            retry_count: 0,
        }
    }
}

impl Request {
    /// 创建 GET 请求。
    pub fn get(url: &str) -> Self {
        Self {
            url: url.to_string(),
            method: Method::Get,
            headers: HashMap::new(),
            body: None,
            meta: Value::Null,
            callback: None,
            spider: None,
            priority: 0,
            depth: 0,
            proxy: None,
            fetch_mode_override: None,
            retry_count: 0,
        }
    }

    /// 创建 POST 请求。
    pub fn post(url: &str, body: Option<String>) -> Self {
        Self {
            url: url.to_string(),
            method: Method::Post,
            headers: HashMap::new(),
            body,
            meta: Value::Null,
            callback: None,
            spider: None,
            priority: 0,
            depth: 0,
            proxy: None,
            fetch_mode_override: None,
            retry_count: 0,
        }
    }

    /// 设置自定义 header。
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    /// 设置元数据。
    pub fn with_meta(mut self, meta: Value) -> Self {
        self.meta = meta;
        self
    }

    /// 设置优先级。
    pub fn with_priority(mut self, p: i32) -> Self {
        self.priority = p;
        self
    }

    /// 设置回调名称。
    pub fn with_callback(mut self, cb: &str) -> Self {
        self.callback = Some(cb.to_string());
        self
    }

    /// 设置 Spider 名称（供 `Engine::run_many` 内部路由使用）。
    pub fn with_spider(mut self, name: impl Into<String>) -> Self {
        self.spider = Some(name.into());
        self
    }

    /// 设置深度。
    pub fn with_depth(mut self, d: u32) -> Self {
        self.depth = d;
        self
    }

    /// 设置代理 URL。
    pub fn with_proxy(mut self, proxy: &str) -> Self {
        self.proxy = Some(proxy.to_string());
        self
    }
}
