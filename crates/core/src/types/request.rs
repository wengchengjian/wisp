//! 传输请求与 Crawl 请求类型。

use super::{FetchMode, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

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

/// 传输请求：Fetcher 真正消费的最小表面。
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
    /// 代理 URL（由中间件设置，FetchClient 读取并应用）。
    #[serde(skip)]
    pub proxy: Option<String>,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: Method::Get,
            headers: HashMap::new(),
            body: None,
            proxy: None,
        }
    }
}

impl Request {
    /// 创建 GET 请求。
    pub fn get(url: &str) -> Self {
        Self {
            url: url.to_string(),
            ..Default::default()
        }
    }

    /// 创建 POST 请求。
    pub fn post(url: &str, body: Option<String>) -> Self {
        Self {
            url: url.to_string(),
            method: Method::Post,
            body,
            ..Default::default()
        }
    }

    /// 设置自定义 header。
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    /// 设置代理 URL。
    pub fn with_proxy(mut self, proxy: &str) -> Self {
        self.proxy = Some(proxy.to_string());
        self
    }
}

/// Crawl 请求：传输请求 + Spider/Engine 状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlRequest {
    /// 传输请求。
    pub request: Request,
    /// Spider 回调名称。
    pub callback: Option<String>,
    /// 多 Spider 模式下所属 Spider 名称（稳定 id）；None 表示尚未绑定。
    #[serde(default)]
    pub spider: Option<String>,
    /// 优先级（Spider 调度用）。
    pub priority: i32,
    /// 深度：起始 URL 为 0，每 follow 一次 +1。
    #[serde(default)]
    pub depth: u32,
    /// 用户自定义元数据（Spider 场景传递深度、回调等）。
    #[serde(with = "meta_serde")]
    pub meta: Value,
    /// 网络错误重试计数（由 engine 维护，中间件只读）。
    #[serde(skip)]
    pub retry_count: u32,
    /// 抓取模式覆盖（由 StealthUpgradeMiddleware 等设置，引擎优先使用此模式）。
    #[serde(skip)]
    pub fetch_mode_override: Option<FetchMode>,
}

impl Default for CrawlRequest {
    fn default() -> Self {
        Self {
            request: Request::default(),
            callback: None,
            spider: None,
            priority: 0,
            depth: 0,
            meta: Value::Null,
            retry_count: 0,
            fetch_mode_override: None,
        }
    }
}

impl CrawlRequest {
    /// 创建 GET Crawl 请求。
    pub fn get(url: &str) -> Self {
        Self {
            request: Request::get(url),
            ..Default::default()
        }
    }

    /// 创建 POST Crawl 请求。
    pub fn post(url: &str, body: Option<String>) -> Self {
        Self {
            request: Request::post(url, body),
            ..Default::default()
        }
    }

    /// 设置自定义 header。
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.request
            .headers
            .insert(key.to_string(), value.to_string());
        self
    }

    /// 设置代理 URL。
    pub fn with_proxy(mut self, proxy: &str) -> Self {
        self.request.proxy = Some(proxy.to_string());
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

    /// 取出传输请求。
    pub fn into_request(self) -> Request {
        self.request
    }
}

impl From<Request> for CrawlRequest {
    fn from(request: Request) -> Self {
        Self {
            request,
            ..Default::default()
        }
    }
}

impl From<&Request> for CrawlRequest {
    fn from(request: &Request) -> Self {
        Self::from(request.clone())
    }
}

impl Deref for CrawlRequest {
    type Target = Request;

    fn deref(&self) -> &Self::Target {
        &self.request
    }
}

impl DerefMut for CrawlRequest {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.request
    }
}
