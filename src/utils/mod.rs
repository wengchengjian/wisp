//! 通用工具函数集合。
//!
//! 将项目内散落的跨模块工具函数统一收归此处，避免重复实现。

pub mod http;
pub mod random;
pub mod url;

pub use http::status_text;
pub use random::rand_suffix;
pub use url::{resolve_href, url_to_filename};
