//! 通用工具函数集合。
//!
//! 将项目内散落的跨模块工具函数统一收归此处，避免重复实现。

pub mod http;
pub mod port;
pub mod random;
pub mod ssrf;
pub mod url;

pub use http::status_text;
pub use port::{find_available_port, get_random_port, is_port_available};
pub use random::rand_suffix;
pub use ssrf::validate_url;
pub use url::{resolve_href, sanitize_url, url_to_filename};
