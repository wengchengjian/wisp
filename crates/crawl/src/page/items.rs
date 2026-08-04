//! Item 收集与页面完成。

use super::*;
use serde::Serialize;

impl Page {
    /// 收集一个 item（自动序列化为 JSON）。
    pub fn item(&mut self, item: impl Serialize) -> &mut Self {
        match serde_json::to_value(item) {
            Ok(value) => self.items.push(value),
            Err(e) => {
                // 不静默写入 Value::Null：序列化失败时跳过该项并告警。
                tracing::warn!("Page::item 序列化失败，跳过该 item: {e}");
            }
        }
        self
    }

    /// 直接收集一个已序列化的 JSON value（避免 `Value` 再次经过序列化克隆）。
    pub fn item_value(&mut self, value: Value) -> &mut Self {
        self.items.push(value);
        self
    }

    /// 已收集的 items。
    pub fn items(&self) -> &[Value] {
        &self.items
    }

    /// 完成处理，产出 `(items, follows)`。
    pub fn finish(self) -> (Vec<Value>, Vec<CrawlRequest>) {
        (self.items, self.follows)
    }
}

impl From<Page> for (Vec<Value>, Vec<CrawlRequest>) {
    fn from(page: Page) -> Self {
        page.finish()
    }
}
