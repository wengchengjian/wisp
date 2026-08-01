//! BrowserHandle RAII。

use tokio::sync::OwnedSemaphorePermit;

use crate::Page;

/// RAII handle：持有 page + permit，Drop 时自动关闭 tab + release permit。
///
/// `page` 用 `Option<Page>` 以便 `Drop` 时 `take()`。正常路径下
/// `fetch_browser` 已显式 `page.close().await`，`target_id` 置 `None`，
/// `Page::Drop` 不会重复关闭；`BrowserHandle::Drop` 只是触发 `Page::Drop`
/// + permit 自动 release。
pub struct BrowserHandle {
    pub(super) page: Option<Page>,
    /// 通过 Drop 释放 permit（回退到 Semaphore），无需显式读取。
    #[allow(dead_code)]
    pub(super) permit: OwnedSemaphorePermit,
}

impl BrowserHandle {
    /// 访问内部 Page。
    pub fn page(&self) -> &Page {
        self.page.as_ref().expect("page must be Some until Drop")
    }

    /// 可变访问内部 Page。
    pub fn page_mut(&mut self) -> &mut Page {
        self.page.as_mut().expect("page must be Some until Drop")
    }
}

impl Drop for BrowserHandle {
    fn drop(&mut self) {
        // 取出 page 触发 Page::Drop（兜底关闭 tab，若已 close 则 no-op）
        // permit 自动 release（OwnedSemaphorePermit::Drop）
        self.page.take();
    }
}
