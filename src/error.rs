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
    /// 浏览器启动失败（找不到可执行文件、进程启动失败等）。
    #[error("Browser launch failed: {0}")]
    LaunchFailed(String),

    /// CDP WebSocket 连接失败。
    #[error("CDP connection error: {0}")]
    CdpConnection(String),

    /// CDP 协议错误（命令执行失败）。
    #[error("CDP protocol error: {0}")]
    CdpProtocol(String),

    /// 页面导航失败。
    #[error("Navigation failed: {0}")]
    NavigationFailed(String),

    /// 元素未找到。
    #[error("Element not found: {selector}")]
    ElementNotFound {
        /// CSS 选择器。
        selector: String,
    },

    /// JavaScript 执行错误。
    #[error("JS evaluation error: {0}")]
    EvalError(String),

    /// 其他浏览器错误。
    #[error("Browser error: {0}")]
    Other(String),
}

// ============================================================================
// 网络错误
// ============================================================================

/// HTTP 请求和网络连接相关错误（支持按类别差异化重试/降级策略）。
#[derive(Debug, Error)]
pub enum NetworkError {
    /// 通用 HTTP 错误。
    #[error("HTTP error: {0}")]
    Http(String),

    /// DNS 解析失败（域名不存在或解析超时）。
    #[error("DNS resolution failed for {host}: {detail}")]
    DnsFailed {
        /// 目标主机名。
        host: String,
        /// 错误详情。
        detail: String,
    },

    /// TCP 连接失败（拒绝/超时/网络不可达）。
    #[error("Connection failed to {host}: {detail}")]
    ConnectionFailed {
        /// 目标主机名。
        host: String,
        /// 错误详情。
        detail: String,
    },

    /// TLS 握手失败（证书无效/协议不匹配）。
    #[error("TLS handshake failed with {host}: {detail}")]
    TlsFailed {
        /// 目标主机名。
        host: String,
        /// 错误详情。
        detail: String,
    },

    /// HTTP 请求超时（连接或读取阶段）。
    #[error("Request to {url} timed out after {timeout_secs}s")]
    RequestTimedOut {
        /// 请求 URL。
        url: String,
        /// 超时时间（秒）。
        timeout_secs: u64,
    },

    /// 代理连接/认证失败。
    #[error("Proxy error: {detail}")]
    ProxyFailed {
        /// 错误详情。
        detail: String,
    },

    /// 响应体超过大小限制。
    #[error("Response body too large: {actual} bytes (limit: {limit} bytes) for {url}")]
    ResponseBodyTooLarge {
        /// 请求 URL。
        url: String,
        /// 实际大小（字节）。
        actual: usize,
        /// 限制大小（字节）。
        limit: usize,
    },

    /// URL 解析失败。
    #[error("URL parse error: {0}")]
    UrlParse(String),
}

// ============================================================================
// 解析 / 数据错误
// ============================================================================

/// HTML 解析、JSON 处理、序列化相关错误。
#[derive(Debug, Error)]
pub enum ParseError {
    /// HTML 解析错误。
    #[error("Parse error: {0}")]
    Html(String),

    /// JSON 处理错误。
    #[error("JSON error: {0}")]
    Json(String),

    /// 序列化错误。
    #[error("Serialize error: {0}")]
    Serialize(String),

    /// 自适应重定位失败。
    #[error("Adaptive relocation failed: {0}")]
    Adaptive(String),
}

// ============================================================================
// MCP 错误
// ============================================================================

/// MCP Server JSON-RPC 交互相关错误。
#[derive(Debug, Error)]
pub enum McpError {
    /// 通用 MCP 错误。
    #[error("MCP error: {0}")]
    General(String),

    /// 未知工具。
    #[error("MCP unknown tool: {0}")]
    UnknownTool(String),
}

// ============================================================================
// 存储错误
// ============================================================================

/// SQLite / 持久化存储相关错误。
#[derive(Debug, Error)]
pub enum StorageError {
    /// 通用存储错误（保留向后兼容，新代码应使用具体变体）。
    #[error("Storage error: {0}")]
    General(String),

    /// 键不存在（namespace + key 定位）。
    #[error("Key not found in namespace {namespace}: {key}")]
    NotFound {
        /// 命名空间（如 "checkpoint"/"element"/"response"）。
        namespace: String,
        /// 键名。
        key: String,
    },

    /// 序列化/反序列化失败。
    #[error("Serialization failed: {0}")]
    Serialization(String),

    /// 后端错误（SQLite/文件系统等底层错误）。
    #[error("Backend error: {0}")]
    Backend(String),

    /// 数据损坏（存储的内容无法解析）。
    #[error("Data corrupted: {0}")]
    Corrupted(String),

    /// IO 错误。
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
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
/// - `Engine(...)` — 引擎状态错误（ND-001-ARCH）
/// - `Config(...)` — 配置 / 使用方式错误
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

    /// ND-001-ARCH：引擎状态错误（如并发 run 同一 Engine）。
    ///
    /// 语义上不属于网络/浏览器/解析等任何领域，应使用此变体。
    #[error("Engine state error: {0}")]
    Engine(String),

    /// 配置 / 使用方式错误（如 `Fetcher::from_client` 创建 Dynamic/Stealth 模式后调用 fetch）。
    #[error("Config error: {0}")]
    Config(String),

    /// 通用超时（跨领域）
    #[error("Timeout: {0}")]
    Timeout(String),

    /// 系统 IO 错误
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Wisp 统一结果类型。
pub type Result<T> = std::result::Result<T, WispError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_error_general_display() {
        let e = StorageError::General("msg".into());
        assert_eq!(e.to_string(), "Storage error: msg");
    }

    #[test]
    fn storage_error_not_found_display() {
        let e = StorageError::NotFound {
            namespace: "checkpoint".into(),
            key: "spider1".into(),
        };
        assert_eq!(
            e.to_string(),
            "Key not found in namespace checkpoint: spider1"
        );
    }

    #[test]
    fn storage_error_serialization_display() {
        let e = StorageError::Serialization("bad json".into());
        assert_eq!(e.to_string(), "Serialization failed: bad json");
    }

    #[test]
    fn storage_error_backend_display() {
        let e = StorageError::Backend("sqlite locked".into());
        assert_eq!(e.to_string(), "Backend error: sqlite locked");
    }

    #[test]
    fn storage_error_corrupted_display() {
        let e = StorageError::Corrupted("invalid magic".into());
        assert_eq!(e.to_string(), "Data corrupted: invalid magic");
    }

    #[test]
    fn storage_error_io_from_std_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let storage_err: StorageError = io_err.into();
        assert!(storage_err.to_string().contains("file missing"));
    }

    #[test]
    fn storage_error_converts_to_wisp_error() {
        let storage_err = StorageError::NotFound {
            namespace: "ns".into(),
            key: "k".into(),
        };
        let wisp_err: WispError = storage_err.into();
        assert!(matches!(wisp_err, WispError::Storage(StorageError::NotFound { .. })));
    }
}
