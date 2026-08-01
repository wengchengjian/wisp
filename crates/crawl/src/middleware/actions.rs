//! 请求/响应/错误阶段中间件动作类型。

use crate::{Request, Response};

// === 请求阶段动作 ===

/// 请求阶段中间件处理结果。
///
/// 请求阶段只允许：继续、修改、跳过、终止、短路响应。
/// `Refetch` 属于响应阶段动作，在请求阶段返回会破坏控制流，因此由类型排除。
#[derive(Debug, Clone)]
pub enum RequestMwAction {
    /// 继续传递给下一个中间件
    Continue,
    /// 已修改请求，继续传递
    Modified,
    /// 跳过此请求（不再继续传递，不发送）
    Skip,
    /// 终止整个爬取，附带原因
    Abort(String),
    /// 短路：直接返回响应，跳过实际网络请求（用于缓存命中）
    Respond(Response),
}

impl PartialEq for RequestMwAction {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Continue, Self::Continue) => true,
            (Self::Modified, Self::Modified) => true,
            (Self::Skip, Self::Skip) => true,
            (Self::Abort(a), Self::Abort(b)) => a == b,
            (Self::Respond(a), Self::Respond(b)) => a.url == b.url,
            _ => false,
        }
    }
}

// === 响应阶段动作 ===

/// 响应阶段中间件处理结果。
///
/// 响应阶段只允许：继续、修改、跳过、终止、重新获取。
/// `Respond` 属于请求阶段动作（缓存短路），在响应阶段返回没有意义，由类型排除。
#[derive(Debug, Clone)]
pub enum ResponseMwAction {
    /// 继续传递给下一个中间件
    Continue,
    /// 已修改响应，继续传递
    Modified,
    /// 跳过此请求（不再继续传递，不处理）
    Skip,
    /// 终止整个爬取，附带原因
    Abort(String),
    /// 用修改后的请求重新获取（用于 Cookie 挑战、JS 重定向等需要"检测→修改→重发"的场景）
    Refetch(Request),
}

impl PartialEq for ResponseMwAction {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Continue, Self::Continue) => true,
            (Self::Modified, Self::Modified) => true,
            (Self::Skip, Self::Skip) => true,
            (Self::Abort(a), Self::Abort(b)) => a == b,
            (Self::Refetch(a), Self::Refetch(b)) => a.url == b.url,
            _ => false,
        }
    }
}

/// 错误处理动作。
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorAction {
    /// 继续传播错误（默认）
    Propagate,
    /// 重试此请求
    Retry,
    /// 忽略错误，跳过此请求
    Ignore,
}
