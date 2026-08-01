//! 浏览器可执行文件查找与启动参数构建。

mod args;
mod resolve;

#[cfg(test)]
mod tests;

pub use args::{build_default_args, build_stealth_args};
pub use resolve::resolve_executable;
