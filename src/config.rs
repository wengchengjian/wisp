use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub headless: bool,
    pub channel: Option<String>,
    pub executable_path: Option<PathBuf>,
    pub user_data_dir: Option<PathBuf>,
    pub no_viewport: bool,
    pub args: Vec<String>,
    pub proxy: Option<ProxyConfig>,
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

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub server: String,
    pub username: Option<String>,
    pub password: Option<String>,
}
