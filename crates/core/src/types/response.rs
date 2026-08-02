//! 统一响应类型 - 所有 Fetcher 模式返回此类型。

use crate::error::{ParseError, Result, WispError};
use crate::types::Request;
use crate::utils::resolve_href;
use serde_json::Value;
use std::collections::HashMap;

/// 统一响应类型 - 所有 Fetcher 模式返回此类型。
///
/// # 示例
///
/// ```rust,no_run
/// use wisp_core::{Request, Response};
///
/// let resp = Response::from_http(
///     200,
///     "https://example.com/".to_string(),
///     Default::default(),
///     b"<h1>Hello</h1>".to_vec(),
///     "text/html; charset=utf-8".to_string(),
///     Request::get("https://example.com/"),
/// );
/// assert_eq!(resp.status, 200);
/// ```
#[derive(Debug, Clone)]
pub struct Response {
    /// HTTP 状态码
    pub status: u16,
    /// 最终 URL（重定向后）
    pub url: String,
    /// 响应头
    pub headers: HashMap<String, String>,
    /// 响应体原始字节
    pub body: Vec<u8>,
    /// 浏览器模式下的页面标题
    pub title: Option<String>,
    /// 浏览器模式下的 cookies
    pub cookies: Vec<String>,
    /// 发起此响应的请求（用于 follow()）
    pub request: Request,
    /// Content-Type 头（用于编码检测）
    pub content_type: String,
    /// 是否来自缓存（缓存命中不算 pages_crawled）。
    #[doc(hidden)]
    pub from_cache: bool,
}

impl Response {
    /// 从所有字段构建（内部使用，如 Engine 组装响应）。
    #[doc(hidden)]
    // 9 参数属于内部组装 API，拆结构体会扩大公共 API 面。
    #[expect(clippy::too_many_arguments)]
    pub fn from_parts(
        status: u16,
        url: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
        title: Option<String>,
        cookies: Vec<String>,
        request: Request,
        content_type: String,
        from_cache: bool,
    ) -> Self {
        Self {
            status,
            url,
            headers,
            body,
            title,
            cookies,
            request,
            content_type,
            from_cache,
        }
    }

    /// 从 HTTP 响应构建。
    pub fn from_http(
        status: u16,
        url: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
        content_type: String,
        request: Request,
    ) -> Self {
        Self {
            status,
            url,
            headers,
            body,
            title: None,
            cookies: Vec::new(),
            request,
            content_type,
            from_cache: false,
        }
    }

    /// 从浏览器响应构建。
    pub fn from_browser(
        status: u16,
        url: String,
        html: String,
        title: String,
        cookies: Vec<String>,
        request: Request,
    ) -> Self {
        Self {
            status,
            url,
            headers: HashMap::new(),
            body: html.into_bytes(),
            title: Some(title),
            cookies,
            request,
            content_type: "text/html; charset=utf-8".to_string(),
            from_cache: false,
        }
    }

    // === 文本/数据 ===

    /// 解码响应体为文本（自动字符集检测）。
    pub fn text(&self) -> Result<String> {
        Ok(crate::encoding::decode(&self.body, &self.content_type))
    }

    /// 解析响应体为 JSON。
    pub fn json(&self) -> Result<Value> {
        let text = self.text()?;
        serde_json::from_str(&text)
            .map_err(|e| WispError::Parse(ParseError::Json(format!("JSON parse: {e}"))))
    }

    /// 状态码是否为 2xx。
    pub fn is_ok(&self) -> bool {
        self.status >= 200 && self.status < 300
    }

    /// 获取页面标题（浏览器模式）。
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    // === 导航 ===

    /// 从当前响应 URL 解析相对链接，创建 GET 请求（depth 自动 +1）。
    pub fn follow(&self, href: &str) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(Request::get(&absolute).with_depth(self.request.depth + 1))
    }

    /// 创建带 callback 的跟随请求（depth 自动 +1）。
    pub fn follow_with(&self, href: &str, callback: &str) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(
            Request::get(&absolute)
                .with_callback(callback)
                .with_depth(self.request.depth + 1),
        )
    }

    /// 创建带 meta 的跟随请求（depth 自动 +1）。
    pub fn follow_meta(&self, href: &str, meta: Value) -> Option<Request> {
        let absolute = resolve_href(&self.url, href)?;
        Some(
            Request::get(&absolute)
                .with_meta(meta)
                .with_depth(self.request.depth + 1),
        )
    }
}
