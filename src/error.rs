use thiserror::Error;

#[derive(Debug, Error)]
pub enum WispError {
    #[error("Browser launch failed: {0}")]
    LaunchFailed(String),

    #[error("CDP connection error: {0}")]
    CdpError(String),

    #[error("Navigation failed: {0}")]
    NavigationFailed(String),

    #[error("Element not found: {selector}")]
    ElementNotFound { selector: String },

    #[error("JS evaluation error: {0}")]
    EvalError(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("CDP error: {0}")]
    CdpProtocol(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Adaptive relocation failed: {0}")]
    AdaptiveError(String),

    #[error("Serialize error: {0}")]
    Serialize(String),

    #[error("MCP error: {0}")]
    McpError(String),

    #[error("MCP unknown tool: {0}")]
    McpUnknownTool(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("JSON error: {0}")]
    JsonError(String),

    #[error("Browser error: {0}")]
    BrowserError(String),

    // === 结构化网络错误（支持按类别差异化重试/降级策略） ===

    /// DNS 解析失败（域名不存在或解析超时）
    #[error("DNS resolution failed for {host}: {detail}")]
    DnsFailed { host: String, detail: String },

    /// TCP 连接失败（拒绝/超时/网络不可达）
    #[error("Connection failed to {host}: {detail}")]
    ConnectionFailed { host: String, detail: String },

    /// TLS 握手失败（证书无效/协议不匹配）
    #[error("TLS handshake failed with {host}: {detail}")]
    TlsFailed { host: String, detail: String },

    /// HTTP 请求超时（连接或读取阶段）
    #[error("Request to {url} timed out after {timeout_secs}s")]
    RequestTimedOut { url: String, timeout_secs: u64 },

    /// 代理连接/认证失败
    #[error("Proxy error: {detail}")]
    ProxyFailed { detail: String },

    /// 响应体超过大小限制
    #[error("Response body too large: {actual} bytes (limit: {limit} bytes) for {url}")]
    ResponseBodyTooLarge { url: String, actual: usize, limit: usize },

    /// URL 解析失败
    #[error("URL parse error: {0}")]
    UrlParse(String),
}

pub type Result<T> = std::result::Result<T, WispError>;
