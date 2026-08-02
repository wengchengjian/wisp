//! SpiderBuilder: 闭包式 Spider 定义，无需手写实现 trait。
//!
//! # 示例
//!
//! ## 简单爬虫（单 default handler）
//!
//! ```rust,no_run
//! use wisp_crawl::SpiderBuilder;
//! use wisp_parser::ResponseExt;
//!
//! let spider = SpiderBuilder::new("quotes")
//!     .start_urls(vec!["https://quotes.toscrape.com/"])
//!     .on("default", async |resp| {
//!         let doc = resp.parse();
//!         let items = doc.select(".quote").iter().map(|q| {
//!             serde_json::json!({ "text": q.select_one(".text").map(|n| n.text()) })
//!         }).collect();
//!         (items, vec![])
//!     })
//!     .build();
//! ```
//!
//! ## 多 callback 路由（列表 → 详情 → 内容）
//!
//! ```rust,no_run
//! use wisp_crawl::SpiderBuilder;
//! use wisp_crawl::stop::MaxPages;
//! use wisp_parser::ResponseExt;
//!
//! let spider = SpiderBuilder::new("pipeline")
//!     .start_urls(vec!["https://example.com/list"])
//!     .on("default", async |resp| {
//!         // 列表页：follow 到 "detail"
//!         let follows: Vec<_> = resp.css(".item a").iter()
//!             .filter_map(|a| resp.follow_with(&a.attr("href").unwrap_or_default(), "detail"))
//!             .collect();
//!         (vec![], follows)
//!     })
//!     .on("detail", async |resp| {
//!         // 详情页：follow 到 "content"
//!         let follows: Vec<_> = resp.css("article a").iter()
//!             .filter_map(|a| resp.follow_with(&a.attr("href").unwrap_or_default(), "content"))
//!             .collect();
//!         (vec![], follows)
//!     })
//!     .on("content", async |resp| {
//!         // 内容页：提取数据
//!         (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
//!     })
//!     .until(MaxPages(1000))
//!     .build();
//! ```

mod closure;
mod spider;

#[cfg(test)]
mod tests;

pub use closure::ClosureSpider;
pub use spider::{Handler, SpiderBuilder};
