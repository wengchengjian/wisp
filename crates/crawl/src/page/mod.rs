//! Page: 单页面处理上下文。
//!
//! 收敛 Spider handler 中最常见的重复流程：解析一次、在文档上多次查询、
//! 收集 item、从链接生成 follow 请求，最后统一产出 `(items, follows)`。

mod items;
mod links;
mod meta;
mod query;

#[cfg(test)]
mod tests;

use serde_json::Value;
use wisp_fetcher::{Request, Response};
use wisp_parser::{Node, ResponseExt};

/// 单页面处理上下文。
///
/// 由 [`crate::SpiderBuilder::on_page`] 创建，构造时解析一次 HTML，
/// 之后可通过 [`Page::doc`] / [`Page::css`] 任意次查询，无需担心
/// `Response::parse()` 的“仅一次”限制。
pub struct Page {
    resp: Response,
    doc: Node,
    items: Vec<Value>,
    follows: Vec<Request>,
}

impl Page {
    /// 从响应创建页面上下文，并立即解析 HTML。
    ///
    /// 注意：`ResponseExt` 的查询方法每次调用都会重新解析；`Page` 只解析一次，
    /// 之后所有查询复用同一份文档。
    pub fn new(resp: Response) -> Self {
        let doc = resp.parse();
        Self {
            resp,
            doc,
            items: Vec::new(),
            follows: Vec::new(),
        }
    }
}
