//! 浏览器启动选项和代理配置。

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// 浏览器启动选项。
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    /// 是否无头模式（无界面）。
    pub headless: bool,
    /// 浏览器通道（chrome / msedge / chromium）。
    pub channel: Option<String>,
    /// 可执行文件路径（不指定则自动查找）。
    pub executable_path: Option<PathBuf>,
    /// 用户数据目录（不指定则使用临时目录）。
    pub user_data_dir: Option<PathBuf>,
    /// 禁用视口模拟。
    pub no_viewport: bool,
    /// 额外启动参数。
    pub args: Vec<String>,
    /// 代理配置。
    pub proxy: Option<ProxyConfig>,
    /// 启动超时。
    pub timeout: Duration,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            headless: false,
            channel: None,
            executable_path: None,
            user_data_dir: None,
            no_viewport: false,
            args: Vec::new(),
            proxy: None,
            timeout: Duration::from_secs(30),
        }
    }
}

/// 代理配置（支持认证）。
#[derive(Clone)]
pub struct ProxyConfig {
    /// 代理服务器地址（如 `http://127.0.0.1:7897`）。
    pub server: String,
    /// 代理用户名（可选）。
    pub username: Option<String>,
    /// 代理密码（可选）。
    pub password: Option<String>,
}

impl fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("server", &self.server)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
}
