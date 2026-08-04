//! wisp 共享基础层：DTO、错误、配置、文本/URL 工具、编码。

pub mod config;
pub mod cookie;
pub mod encoding;
pub mod error;
pub mod stealth;
pub mod text;
pub mod types;
pub mod utils;

pub use cookie::{Cookie, CookieJar, MockCookieJar};
pub use stealth::{ChallengeType, TurnstileConfig};
pub use types::{CrawlRequest, FetchMode, Method, Request, Response, ResponseParts};
