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
}

pub type Result<T> = std::result::Result<T, WispError>;
