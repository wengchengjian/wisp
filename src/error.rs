//! 分类错误体系。
//!
//! 顶层 `WispError` 按领域分为 5 个子枚举：
//! - [`BrowserError`] — 浏览器启动、CDP 通信、页面操作
//! - [`NetworkError`] — HTTP 请求、DNS/TLS/代理、超时
//! - [`ParseError`] — HTML/JSON/序列化解析
//! - [`McpError`] — MCP 协议交互
//! - [`StorageError`] — SQLite 持久化
//!
//! 另有 `Timeout` 和 `Io` 作为跨领域通用变体保留在顶层。

use thiserror::Error;

// ============================================================================
// 浏览器 / CDP 错误
// ============================================================================

/// 浏览器生命周期和 CDP 通信相关错误。
#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("Browser launch failed: {0}")]
    LaunchFailed(String),

    #[error("CDP connection error: {0}")]
    CdpConnection(String),

    #[error("CDP protocol error: {0}")]
    CdpProtocol(String),

    #[error("Navigation failed: {0}")]
    NavigationFailed(String),

    #[error("Element not found: {selector}")]
    ElementNotFound { selector: String },

    #[error("JS evaluation error: {0}")]
    EvalError(String),

    #[error("Browser error: {0}")]
    Other(String),
}

// ============================================================================
// 网络错误
// ============================================================================

/// HTTP 请求和网络连接相关错误（支持按类别差异化重试/降级策略）。
#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("HTTP error: {0}")]
    Http(String),

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

// ============================================================================
// 解析 / 数据错误
// ============================================================================

/// HTML 解析、JSON 处理、序列化相关错误。
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Parse error: {0}")]
    Html(String),

    #[error("JSON error: {0}")]
    Json(String),

    #[error("Serialize error: {0}")]
    Serialize(String),

    #[error("Adaptive relocation failed: {0}")]
    Adaptive(String),
}

// ============================================================================
// MCP 错误
// ============================================================================

/// MCP Server JSON-RPC 交互相关错误。
#[derive(Debug, Error)]
pub enum McpError {
    #[error("MCP error: {0}")]
    General(String),

    #[error("MCP unknown tool: {0}")]
    UnknownTool(String),
}

// ============================================================================
// 存储错误
// ============================================================================

/// SQLite / 持久化存储相关错误。
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Storage error: {0}")]
    General(String),
}

// ============================================================================
// 顶层统一错误
// ============================================================================

/// Wisp 统一错误类型。
///
/// 按领域分类为子枚举，通过 `#[from]` 支持 `?` 自动转换：
/// - `Browser(...)` — 浏览器 / CDP
/// - `Network(...)` — HTTP / 网络连接
/// - `Parse(...)` — 解析 / 序列化
/// - `Mcp(...)` — MCP 协议
/// - `Storage(...)` — 存储
/// - `Timeout` / `Io` — 跨领域通用
#[derive(Debug, Error)]
pub enum WispError {
    /// 浏览器 / CDP 错误
    #[error(transparent)]
    Browser(#[from] BrowserError),

    /// 网络 / HTTP 错误
    #[error(transparent)]
    Network(#[from] NetworkError),

    /// 解析 / 数据错误
    #[error(transparent)]
    Parse(#[from] ParseError),

    /// MCP 协议错误
    #[error(transparent)]
    Mcp(#[from] McpError),

    /// 存储错误
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// 通用超时（跨领域）
    #[error("Timeout: {0}")]
    Timeout(String),

    /// 系统 IO 错误
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, WispError>;
