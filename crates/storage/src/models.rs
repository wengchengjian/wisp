//! 存储层公共数据类型。

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 可缓存的响应数据（`Response` 的可序列化子集）。
///
/// 不含 `request` 字段——命中时由 `CacheMiddleware` 用当前请求重建完整 `Response`。
/// `cached_at` + `ttl` 配对决定过期时间。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    /// HTTP 状态码。
    pub status: u16,
    /// 响应头。
    pub headers: std::collections::HashMap<String, String>,
    /// 响应体。
    pub body: Vec<u8>,
    /// 内容类型。
    pub content_type: String,
    /// 缓存时刻（Unix 秒）。
    pub cached_at: i64,
    /// 有效期。`None` 表示永不过期。
    pub ttl: Option<Duration>,
}

impl CachedResponse {
    /// 是否已过期（基于 `cached_at` + `ttl` 与当前时间比较）。
    pub fn is_expired(&self) -> bool {
        match self.ttl {
            Some(ttl) => {
                let now = chrono::Utc::now().timestamp();
                now > self.cached_at + ttl.as_secs() as i64
            }
            None => false,
        }
    }
}

/// Element snapshot 行（存储层不感知 `parser::Node`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementSnapshotRow {
    /// 元素标签名。
    pub tag: String,
    /// 元素属性。
    pub attrs: serde_json::Value,
    /// 文本预览。
    pub text_preview: String,
    /// 祖先路径。
    pub ancestor_path: serde_json::Value,
    /// 兄弟标签。
    pub sibling_tags: serde_json::Value,
    /// 在父节点中的位置。
    pub position_in_parent: i64,
    /// 父节点标签。
    pub parent_tag: String,
    /// 父节点属性。
    pub parent_attrs: serde_json::Value,
    /// 捕获时间（Unix 秒）。
    pub captured_at: i64,
}
