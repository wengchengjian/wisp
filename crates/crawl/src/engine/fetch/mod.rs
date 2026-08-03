//! Engine 子模块：fetch。

use super::*;

mod dispatch;
mod page;

pub(crate) use dispatch::fetch_dispatch;
