//! CF 会话条目。

/// CF 会话条目：cookie + UA + sec-ch-ua 绑定存储。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CfSession {
    /// CDP 返回的原始 cookie JSON 数组（含 name/value/domain/path/secure/httpOnly/sameSite 等）。
    pub cookies: Vec<serde_json::Value>,
    /// 浏览器实际 UA（CF 挑战解决时捕获，复用给后续 HTTP 请求）。
    pub ua: String,
    /// 浏览器实际发送的 sec-ch-ua 头（CF 挑战解决时捕获，复用给后续 HTTP 请求）。
    ///
    /// 不能靠手动构造：不同 Chrome 版本/平台的 brand 顺序与 GREASE 值不同，
    /// 与 UA 不一致会被 CF 判定会话无效（403）。须从浏览器侧捕获真实值。
    #[serde(default)]
    pub sec_ch_ua: String,
    /// Unix 时间戳（秒），用于文件加载时判断过期。
    pub saved_at: i64,
}

impl CfSession {
    /// 创建空会话（saved_at 置为当前时间）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            cookies: Vec::new(),
            ua: String::new(),
            sec_ch_ua: String::new(),
            saved_at: chrono::Utc::now().timestamp(),
        }
    }
}

impl Default for CfSession {
    fn default() -> Self {
        Self::new()
    }
}
