//! robots.txt parsing and caching.
//!
//! 并发设计：读写分离 + single-flight + negative cache with TTL。

mod cache;
mod parser;

#[cfg(test)]
mod tests;

pub use cache::RobotsCache;
pub use parser::{RobotsRules, parse_robots_text};
