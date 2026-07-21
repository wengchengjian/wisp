//! Multi-session Spider support.
//!
//! 鍏佽鍦ㄥ崟涓?Spider 涓娇鐢ㄥ绉?Fetcher 绫诲瀷锛堝揩閫?HTTP / 闅愯韩娴忚鍣級锛?
//! 閫氳繃 session ID 璺敱璇锋眰鍒颁笉鍚岀殑浼氳瘽銆?
//!
//! # 绀轰緥
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

/// Fetcher 绫诲瀷鏋氫妇銆?
#[derive(Debug, Clone)]
pub enum FetcherType {
    /// 蹇€?HTTP 璇锋眰锛坵req TLS 鎸囩汗妯℃嫙锛夈€?
    Http(http::Config),
    /// 闅愯韩娴忚鍣ㄦā寮忥紙閫氳繃 Scraper 缁曡繃 CF锛夈€?
    /// 瀛樺偍浠ｇ悊鍜?headless 閰嶇疆銆?
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

/// 澶氫細璇濈鐞嗗櫒銆?
///
/// 绠＄悊澶氫釜鍛藉悕鐨?Fetcher 浼氳瘽锛孲pider 鍙€氳繃 session ID 璺敱璇锋眰銆?
#[derive(Default)]
pub struct SessionManager {
    sessions: HashMap<String, FetcherType>,
}

impl SessionManager {
    /// 鍒涘缓绌虹殑浼氳瘽绠＄悊鍣ㄣ€?
    pub fn new() -> Self {
        Self { sessions: HashMap::new() }
    }

    /// 娣诲姞涓€涓懡鍚嶄細璇濄€?
    pub fn add(&mut self, id: &str, fetcher: FetcherType) {
        self.sessions.insert(id.to_string(), fetcher);
    }

    /// 鑾峰彇鎸囧畾 ID 鐨勪細璇濋厤缃€?
    pub fn get(&self, id: &str) -> Option<&FetcherType> {
        self.sessions.get(id)
    }

    /// 鑾峰彇榛樿浼氳瘽锛?default"锛夈€?
    pub fn default_session(&self) -> Option<&FetcherType> {
        self.sessions.get("default")
    }

    /// 浼氳瘽鏁伴噺銆?
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// 鎵€鏈変細璇?ID 鍒楄〃銆?
    pub fn session_ids(&self) -> Vec<&str> {
        self.sessions.keys().map(|k| k.as_str()).collect()
    }
}

/// SpiderRequest 鎵╁睍锛氭惡甯?session ID銆?
///
/// 閫氳繃 SpiderRequest.meta 涓殑 "__sid" 瀛楁浼犻€掋€?
pub fn request_with_session(mut req: super::SpiderRequest, sid: &str) -> super::SpiderRequest {
    req.meta = serde_json::json!({ "__sid": sid });
    req
}

/// 浠?SpiderRequest 鎻愬彇 session ID銆?
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
    fn test_session_id_default() {
        let req = super::super::SpiderRequest::get("https://example.com");
        assert_eq!(session_id_of(&req), "default");
    }
}
