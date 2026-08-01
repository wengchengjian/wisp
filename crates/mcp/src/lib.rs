//! MCP (Model Context Protocol) server over stdio JSON-RPC 2.0.
//!
//! 工具定义在 tools 模块，协议与服务实现见 protocol/server 子模块。

pub mod tools;

mod protocol;
mod server;

pub use protocol::{Tool, TOOLS};
pub use server::serve;

#[cfg(test)]
mod tests;
