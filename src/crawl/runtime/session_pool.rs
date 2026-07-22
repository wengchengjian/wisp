//! Session 健康追踪 — 借鉴 Crawlee SessionPool 设计。
//!
//! 每个 Session 绑定一个 proxy + UA + cookies，通过 error_score 追踪健康度。
//! 自动淘汰被封的代理/会话，保持 IP 健康度，减少无效重试。
//!
//! # 与 ProxyPool 关系
//!
//! SessionPool 是 ProxyPool 的上层封装。ProxyPool 保留作为底层代理列表源，
//! SessionPool 在其基础上增加健康追踪、自动轮换、退休机制。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Session 配置。
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// 最大使用次数（超过后退休）
    pub max_usage: u32,
    /// 最大存活时间
    pub max_lifetime: Duration,
    /// 错误分阈值（超过后退休）
    pub max_error_score: f64,
    /// 每次 mark_bad 增加的错误分
    pub error_score_increment: f64,
    /// 每次 mark_good 的衰减系数
    pub good_decay_factor: f64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_usage: 50,
            max_lifetime: Duration::from_secs(300),
            max_error_score: 3.0,
            error_score_increment: 1.0,
            good_decay_factor: 0.8,
        }
    }
}

/// 单个会话：绑定 proxy + UA + cookies + 健康状态。
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub proxy: Option<String>,
    pub cookies: HashMap<String, String>,
    pub user_agent: String,
    pub error_score: f64,
    pub usage_count: u32,
    pub max_usage: u32,
    pub created_at: Instant,
    pub max_lifetime: Duration,
    max_error_score: f64,
    error_score_increment: f64,
    good_decay_factor: f64,
}

impl Session {
    /// 创建新 Session。
    pub fn new(id: String, proxy: Option<String>, user_agent: String, config: &SessionConfig) -> Self {
        Self {
            id,
            proxy,
            cookies: HashMap::new(),
            user_agent,
            error_score: 0.0,
            usage_count: 0,
            max_usage: config.max_usage,
            created_at: Instant::now(),
            max_lifetime: config.max_lifetime,
            max_error_score: config.max_error_score,
            error_score_increment: config.error_score_increment,
            good_decay_factor: config.good_decay_factor,
        }
    }

    /// 标记成功：错误分衰减。
    pub fn mark_good(&mut self) {
        self.error_score *= self.good_decay_factor;
        self.usage_count += 1;
    }

    /// 标记失败：错误分增加。
    pub fn mark_bad(&mut self) {
        self.error_score += self.error_score_increment;
        self.usage_count += 1;
    }

    /// 是否已退休（错误分过高 / 使用次数超限 / 存活超时）。
    pub fn is_retired(&self) -> bool {
        self.error_score >= self.max_error_score
            || self.usage_count >= self.max_usage
            || self.created_at.elapsed() > self.max_lifetime
    }
}

/// Session 池：管理一组 Session 的生命周期。
pub struct SessionPool {
    sessions: Mutex<Vec<Session>>,
    max_size: usize,
    config: SessionConfig,
    proxy_source: Option<Arc<crate::proxy::ProxyPool>>,
    ua_list: Vec<String>,
    next_id: std::sync::atomic::AtomicUsize,
}

impl SessionPool {
    /// 创建 Session 池。
    ///
    /// - `max_size`: 池中最大 Session 数
    /// - `config`: Session 配置
    /// - `proxy_source`: 可选的代理池（用于为新 Session 分配代理）
    pub fn new(
        max_size: usize,
        config: SessionConfig,
        proxy_source: Option<Arc<crate::proxy::ProxyPool>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(Vec::new()),
            max_size,
            config,
            proxy_source,
            ua_list: default_ua_list(),
            next_id: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// 获取一个健康 Session（选取 error_score 最低的非退休 Session）。
    ///
    /// 若池中无可用 Session 且未达上限，自动创建新 Session。
    pub async fn acquire(&self) -> Option<SessionGuard> {
        let mut sessions = self.sessions.lock().await;

        // 清理退休 Session
        sessions.retain(|s| !s.is_retired());

        // 选取 error_score 最低的
        let best_idx = sessions
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.error_score.partial_cmp(&b.error_score).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx);

        if let Some(idx) = best_idx {
            let session = sessions[idx].clone();
            return Some(SessionGuard {
                session,
                pool_sessions: &self.sessions as *const _ as usize, // placeholder
                session_id: sessions[idx].id.clone(),
            });
        }

        // 创建新 Session
        if sessions.len() < self.max_size {
            let id = format!("session-{}", self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            let proxy = self.proxy_source.as_ref().and_then(|p| p.next());
            let ua = self.ua_list[self.next_id.load(std::sync::atomic::Ordering::Relaxed) % self.ua_list.len()].clone();
            let session = Session::new(id.clone(), proxy, ua, &self.config);
            sessions.push(session.clone());
            return Some(SessionGuard {
                session,
                pool_sessions: 0,
                session_id: id,
            });
        }

        None
    }

    /// 标记 Session 成功。
    pub async fn mark_good(&self, id: &str) {
        let mut sessions = self.sessions.lock().await;
        if let Some(s) = sessions.iter_mut().find(|s| s.id == id) {
            s.mark_good();
        }
    }

    /// 标记 Session 失败。
    pub async fn mark_bad(&self, id: &str) {
        let mut sessions = self.sessions.lock().await;
        if let Some(s) = sessions.iter_mut().find(|s| s.id == id) {
            s.mark_bad();
        }
    }

    /// 当前池中 Session 数量。
    pub async fn size(&self) -> usize {
        self.sessions.lock().await.len()
    }

    /// 清理退休 Session。
    pub async fn maintain(&self) {
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|s| !s.is_retired());
    }
}

/// Session 获取守卫（包含 Session 快照）。
pub struct SessionGuard {
    pub session: Session,
    #[allow(dead_code)]
    pool_sessions: usize,
    pub session_id: String,
}

/// 默认 UA 列表。
fn default_ua_list() -> Vec<String> {
    vec![
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".into(),
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".into(),
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:132.0) Gecko/20100101 Firefox/132.0".into(),
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_mark_good_bad() {
        let config = SessionConfig::default();
        let mut session = Session::new("s1".into(), None, "UA".into(), &config);

        session.mark_bad();
        assert_eq!(session.error_score, 1.0);
        assert_eq!(session.usage_count, 1);

        session.mark_good();
        assert!(session.error_score < 1.0); // 衰减
        assert_eq!(session.usage_count, 2);
    }

    #[test]
    fn test_session_retirement() {
        let config = SessionConfig {
            max_error_score: 2.0,
            ..Default::default()
        };
        let mut session = Session::new("s1".into(), None, "UA".into(), &config);

        assert!(!session.is_retired());
        session.mark_bad();
        session.mark_bad();
        assert!(session.is_retired()); // error_score >= 2.0
    }

    #[test]
    fn test_session_max_usage_retirement() {
        let config = SessionConfig {
            max_usage: 2,
            ..Default::default()
        };
        let mut session = Session::new("s1".into(), None, "UA".into(), &config);

        session.mark_good();
        assert!(!session.is_retired());
        session.mark_good();
        assert!(session.is_retired()); // usage_count >= max_usage
    }

    #[tokio::test]
    async fn test_session_pool_acquire() {
        let pool = SessionPool::new(5, SessionConfig::default(), None);
        let guard = pool.acquire().await;
        assert!(guard.is_some());
        assert_eq!(pool.size().await, 1);
    }

    #[tokio::test]
    async fn test_session_pool_mark_bad_retires() {
        let config = SessionConfig {
            max_error_score: 1.0,
            ..Default::default()
        };
        let pool = SessionPool::new(5, config, None);

        let guard = pool.acquire().await.unwrap();
        let id = guard.session_id.clone();
        pool.mark_bad(&id).await;

        // Session 应已退休，maintain 后池为空
        pool.maintain().await;
        assert_eq!(pool.size().await, 0);
    }
}
