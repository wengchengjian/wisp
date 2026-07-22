//! Multi-session Spider support.
//!
//! 允许在单个 Spider 中使用多种 Fetcher 类型（快速 HTTP / 隐身浏览器）。
//! 通过 session ID 路由请求到不同的会话。
//!
//! # 示例
//!
//! ```rust,no_run
//! use wisp::crawl::session::{SessionManager, FetcherType};
//! use wisp::http;
//!
//! let mut mgr = SessionManager::new();
//! mgr.add("fast", FetcherType::Http(http::Config::default()));
//! mgr.add("stealth", FetcherType::Http(http::Config {
//!     proxy: Some("http://127.0.0.1:7897".into()),
//!     ..Default::default()
//! }));
//! ```

use std::collections::HashMap;
use crate::http;

/// Fetcher 类型枚举。
#[derive(Debug, Clone)]
pub enum FetcherType {
    /// 快速 HTTP 请求（reqwest TLS 指纹模拟）。
    Http(http::Config),
    /// 隐身浏览器模式（通过 Scraper 绕过 CF）。
    /// 存储代理和 headless 配置。
    Stealth {
        headless: bool,
        proxy: Option<String>,
        challenge_timeout_secs: u64,
    },
}

impl Default for FetcherType {
    fn default() -> Self {
        FetcherType::Http(http::Config::default())
    }
}

/// 多会话管理器。
///
/// 管理多个命名的 Fetcher 会话，Spider 可通过 session ID 路由请求。
#[derive(Default)]
pub struct SessionManager {
    sessions: HashMap<String, FetcherType>,
}

impl SessionManager {
    /// 创建空的会话管理器。
    pub fn new() -> Self {
        Self { sessions: HashMap::new() }
    }

    /// 添加一个命名会话。
    pub fn add(&mut self, id: &str, fetcher: FetcherType) {
        self.sessions.insert(id.to_string(), fetcher);
    }

    /// 获取指定 ID 的会话配置。
    pub fn get(&self, id: &str) -> Option<&FetcherType> {
        self.sessions.get(id)
    }

    /// 获取默认会话（"default"）。
    pub fn default_session(&self) -> Option<&FetcherType> {
        self.sessions.get("default")
    }

    /// 会话数量。
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// 所有会话 ID 列表。
    pub fn session_ids(&self) -> Vec<&str> {
        self.sessions.keys().map(|k| k.as_str()).collect()
    }
}

/// SpiderRequest 扩展：携带 session ID。
///
/// 通过 SpiderRequest.meta 中的 "__sid" 字段传递。
pub fn request_with_session(mut req: super::SpiderRequest, sid: &str) -> super::SpiderRequest {
    // 合并而非覆盖：保留原有 meta（如分页信息），仅注入/更新 __sid
    if !req.meta.is_object() {
        req.meta = serde_json::json!({});
    }
    if let Some(obj) = req.meta.as_object_mut() {
        obj.insert("__sid".to_string(), serde_json::Value::String(sid.to_string()));
    }
    req
}

/// 从 SpiderRequest 提取 session ID。
pub fn session_id_of(req: &super::SpiderRequest) -> &str {
    req.meta.get("__sid")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_manager_basic() {
        let mut mgr = SessionManager::new();
        assert!(mgr.is_empty());

        mgr.add("fast", FetcherType::Http(http::Config::default()));
        mgr.add("stealth", FetcherType::Stealth {
            headless: true,
            proxy: Some("http://127.0.0.1:7897".into()),
            challenge_timeout_secs: 60,
        });

        assert_eq!(mgr.len(), 2);
        assert!(mgr.get("fast").is_some());
        assert!(mgr.get("stealth").is_some());
        assert!(mgr.get("nonexistent").is_none());
    }

    #[test]
    fn test_session_ids() {
        let mut mgr = SessionManager::new();
        mgr.add("a", FetcherType::default());
        mgr.add("b", FetcherType::default());

        let ids = mgr.session_ids();
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
    }

    #[test]
    fn test_request_with_session() {
        let req = super::super::SpiderRequest::get("https://example.com");
        let req = request_with_session(req, "stealth");
        assert_eq!(session_id_of(&req), "stealth");
    }

    #[test]
    fn test_request_with_session_preserves_meta() {
        let req = super::super::SpiderRequest::get("https://example.com")
            .with_meta(serde_json::json!({"page": 2, "category": "books"}));
        let req = request_with_session(req, "stealth");
        assert_eq!(session_id_of(&req), "stealth");
        assert_eq!(req.meta["page"], 2);
        assert_eq!(req.meta["category"], "books");
    }

    #[test]
    fn test_session_id_default() {
        let req = super::super::SpiderRequest::get("https://example.com");
        assert_eq!(session_id_of(&req), "default");
    }
}
