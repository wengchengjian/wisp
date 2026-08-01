//! Builtin 中间件子模块：challenge。

use async_trait::async_trait;

use super::{CrawlContext, Middleware, ResponseMwAction};
use crate::Response;

// === 响应挑战类 ===

/// Cookie 挑战中间件：自动解决多步 Set-Cookie + JS 重定向类反爬。
///
/// 检测特征：403 + Set-Cookie + body 极短（< 200 字节）。
/// 解决方式：累积 cookie 并通过 `ResponseMwAction::Refetch` 重新获取。
pub struct CookieChallengeMiddleware {
    /// 最大累积轮数（默认 3）
    max_rounds: usize,
}

impl CookieChallengeMiddleware {
    /// 创建 Cookie 挑战中间件。
    pub fn new(max_rounds: usize) -> Self {
        Self { max_rounds }
    }
}

impl Default for CookieChallengeMiddleware {
    fn default() -> Self {
        Self { max_rounds: 3 }
    }
}

#[async_trait]
impl Middleware for CookieChallengeMiddleware {
    fn priority(&self) -> u32 {
        50
    }

    async fn process_response(&self, resp: &mut Response, _ctx: &CrawlContext) -> ResponseMwAction {
        if resp.status != 403 || resp.body.len() >= 200 {
            return ResponseMwAction::Continue;
        }
        let set_cookie = match resp.headers.get("set-cookie") {
            Some(sc) => sc.clone(),
            None => return ResponseMwAction::Continue,
        };
        let cookie_pair = set_cookie.split(';').next().unwrap_or("").to_string();
        if cookie_pair.is_empty() {
            return ResponseMwAction::Continue;
        }
        let existing = resp
            .request
            .headers
            .get("Cookie")
            .cloned()
            .unwrap_or_default();
        let new_cookie = if existing.is_empty() {
            cookie_pair
        } else {
            if existing.contains(&cookie_pair) {
                return ResponseMwAction::Continue;
            }
            format!("{}; {}", existing, cookie_pair)
        };
        let cookie_count = new_cookie.matches("; ").count() + 1;
        if cookie_count > self.max_rounds {
            return ResponseMwAction::Continue;
        }
        let mut new_req = resp.request.clone();
        new_req.headers.insert("Cookie".to_string(), new_cookie);
        ResponseMwAction::Refetch(new_req)
    }
}
