//! HTTP client request execution and response conversion.

mod error;
mod headers;
mod request;
mod response;

use crate::Config;
use wisp_core::error::Result;

/// HTTP client for fetching web pages.
#[derive(Clone)]
pub struct Client {
    pub(crate) http: wreq::Client,
    pub(crate) config: Config,
}

impl Client {
    /// 创建客户端构建器。
    pub fn builder() -> crate::ClientBuilder {
        crate::ClientBuilder::new()
    }

    /// Create a client with default config.
    pub fn new() -> Result<Self> {
        crate::ClientBuilder::new().build()
    }

    /// Create a client from a complete config.
    pub fn from_config(config: crate::Config) -> Result<Self> {
        crate::ClientBuilder::from_config(config).build()
    }

    /// 获取配置引用（供 Engine 代理轮换时读取 timeout 等参数）。
    pub fn config_ref(&self) -> &Config {
        &self.config
    }
}
