//! 外部配置文件支持。
//!
//! 从项目根目录 `wisp.toml` 加载配置，优先级：代码显式设置 > wisp.toml > 默认值。
//!
//! # 示例 wisp.toml
//!
//! ```toml
//! [engine]
//! max_concurrent = 8
//! max_pages = 1000
//! max_refetch_rounds = 5
//! timeout_secs = 30
//!
//! [proxy]
//! pool = ["http://127.0.0.1:7897"]
//! strategy = "sequential"
//!
//! [http]
//! user_agent = "Mozilla/5.0 ... Chrome/136.0.0.0 ..."
//! emulation = "chrome136"
//!
//! [stealth]
//! headless = true
//! challenge_timeout_secs = 30
//! human_mode = true
//! ```

use serde::Deserialize;
use std::path::Path;

/// Wisp 全局配置（对应 wisp.toml）。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct WispConfig {
    pub engine: EngineConfig,
    pub proxy: ProxyConfig,
    pub http: HttpConfig,
    pub stealth: StealthConfig,
}

/// 引擎级配置。
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    pub max_concurrent: usize,
    pub max_pages: usize,
    pub max_refetch_rounds: usize,
    pub timeout_secs: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 8,
            max_pages: 1000,
            max_refetch_rounds: 5,
            timeout_secs: 30,
        }
    }
}

/// 代理配置。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ProxyConfig {
    /// 代理池 URL 列表
    pub pool: Vec<String>,
    /// 轮换策略：sequential | random | sticky
    pub strategy: String,
}

/// HTTP 客户端配置。
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    pub user_agent: Option<String>,
    /// TLS 指纹模拟：chrome136 | firefox128 | safari18
    pub emulation: Option<String>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            user_agent: None,
            emulation: Some("chrome136".to_string()),
        }
    }
}

/// Stealth 模式配置。
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StealthConfig {
    pub headless: bool,
    pub challenge_timeout_secs: u64,
    pub human_mode: bool,
}

impl Default for StealthConfig {
    fn default() -> Self {
        Self {
            headless: true,
            challenge_timeout_secs: 30,
            human_mode: true,
        }
    }
}

impl WispConfig {
    /// 从指定路径加载配置文件。文件不存在则返回全默认值。
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                toml::from_str(&content).unwrap_or_else(|e| {
                    tracing::warn!("wisp.toml 解析失败，使用默认配置: {}", e);
                    Self::default()
                })
            }
            Err(_) => Self::default(),
        }
    }

    /// 尝试从当前目录加载 `wisp.toml`。
    pub fn load_default() -> Self {
        Self::load(Path::new("wisp.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = WispConfig::default();
        assert_eq!(config.engine.max_concurrent, 8);
        assert_eq!(config.engine.max_pages, 1000);
        assert_eq!(config.engine.max_refetch_rounds, 5);
        assert_eq!(config.engine.timeout_secs, 30);
        assert!(config.proxy.pool.is_empty());
        assert_eq!(config.stealth.headless, true);
    }

    #[test]
    fn test_parse_toml() {
        let toml_str = r#"
[engine]
max_concurrent = 4
max_pages = 200

[proxy]
pool = ["http://127.0.0.1:7897"]
strategy = "random"

[stealth]
headless = false
"#;
        let config: WispConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.engine.max_concurrent, 4);
        assert_eq!(config.engine.max_pages, 200);
        assert_eq!(config.engine.max_refetch_rounds, 5); // 默认值
        assert_eq!(config.proxy.pool, vec!["http://127.0.0.1:7897"]);
        assert_eq!(config.proxy.strategy, "random");
        assert_eq!(config.stealth.headless, false);
        assert_eq!(config.stealth.human_mode, true); // 默认值
    }

    #[test]
    fn test_load_nonexistent_file() {
        let config = WispConfig::load(Path::new("/nonexistent/wisp.toml"));
        assert_eq!(config.engine.max_concurrent, 8);
    }
}
