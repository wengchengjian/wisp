//! Engine 子模块：fetch。

use super::*;

mod dispatch;
mod page;

pub(crate) use dispatch::fetch_dispatch;
pub use page::{fetch_page, fetch_page_inner};
