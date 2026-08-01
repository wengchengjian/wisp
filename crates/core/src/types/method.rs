//! HTTP 方法与抓取模式枚举。

use serde::{Deserialize, Serialize};

/// HTTP 方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    /// GET 请求。
    Get,
    /// POST 请求。
    Post,
    /// PUT 请求。
    Put,
    /// DELETE 请求。
    Delete,
}

impl Method {
    /// 返回标准 HTTP 动词字符串（大写）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

/// 抓取模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchMode {
    /// 快速 HTTP（TLS 指纹模拟，无浏览器）。成本最低，毫秒级。
    Http,
    /// 浏览器渲染（JS 执行，无 CF 绕过）。中等成本，秒级。
    Dynamic,
    /// 隐身浏览器（CF bypass + 人类行为模拟）。最高成本，秒级。
    Stealth,
    /// 自动模式：先 HTTP，中间件驱动升级。
    Auto,
}
