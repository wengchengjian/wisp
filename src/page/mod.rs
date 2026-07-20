use chromiumoxide::Page as CdpPage;

use crate::error::Result;

/// A browser page (tab) with anti-detection patches.
pub struct Page {
    #[allow(dead_code)]
    pub(crate) inner: CdpPage,
}

impl Page {
    pub(crate) async fn new(inner: CdpPage) -> Result<Self> {
        Ok(Self { inner })
    }
}
