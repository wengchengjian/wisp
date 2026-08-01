//! CF 会话条目。

/// CF 会话条目：cookie + UA 绑定存储。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CfSession {
    /// CDP 返回的原始 cookie JSON 数组（含 name/value/domain/path/secure/httpOnly/sameSite 等）。
    pub cookies: Vec<serde_json::Value>,
    /// 浏览器实际 UA（CF 挑战解决时捕获，复用给后续 HTTP 请求）。
    pub ua: String,
    /// Unix 时间戳（秒），用于文件加载时判断过期。
    pub saved_at: i64,
}
