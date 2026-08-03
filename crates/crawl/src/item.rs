//! Item 领域类型：payload + 来源元数据 + 稳定 id。

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 爬取产物。
///
/// `T` 默认是 `serde_json::Value`，engine seam 只使用 `Item<Value>`；
/// 用户可在 pipeline 边界用 [`Item::try_typed`] 转成类型化视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item<T = Value> {
    value: T,
    source_url: String,
    spider: String,
    callback: Option<String>,
    id: String,
}

impl Item<Value> {
    /// 创建 item：来源元数据由 engine 在交付 seam 附着，id 使用稳定哈希。
    pub fn new(value: Value, source_url: &str, spider: &str, callback: Option<&str>) -> Self {
        let id = compute_id(&value, source_url, callback);
        Self {
            value,
            source_url: source_url.to_string(),
            spider: spider.to_string(),
            callback: callback.map(str::to_string),
            id,
        }
    }

    /// payload 借用视图。
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// 取出 payload。
    pub fn into_value(self) -> Value {
        self.value
    }

    /// 替换 payload 并重算 id。
    pub fn with_value(&mut self, value: Value) {
        self.id = compute_id(&value, &self.source_url, self.callback.as_deref());
        self.value = value;
    }

    /// 消费式替换 payload 并重算 id。
    pub fn map_value(mut self, f: impl FnOnce(Value) -> Value) -> Self {
        let value = f(self.value);
        self.value = value;
        self.id = compute_id(&self.value, &self.source_url, self.callback.as_deref());
        self
    }

    /// 借用反序列化为类型化视图，不消耗或克隆 payload。
    pub fn try_typed<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        T::deserialize(&self.value)
    }

    /// 来源 URL。
    pub fn source_url(&self) -> &str {
        &self.source_url
    }

    /// 产出该 item 的 Spider 名称。
    pub fn spider(&self) -> &str {
        &self.spider
    }

    /// 产出该 item 的 callback（无 callback 时为 None）。
    pub fn callback(&self) -> Option<&str> {
        self.callback.as_deref()
    }

    /// 稳定 id。
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl<T> Item<T> {
    /// payload 借用视图（泛型版本）。
    pub fn value_ref(&self) -> &T {
        &self.value
    }

    /// 取出 payload（泛型版本）。
    pub fn into_payload(self) -> T {
        self.value
    }
}

fn compute_id(value: &Value, source_url: &str, callback: Option<&str>) -> String {
    use sha2::{Digest, Sha256};

    let canonical = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update((source_url.len() as u64).to_le_bytes());
    hasher.update(source_url.as_bytes());
    hasher.update((callback.unwrap_or("default").len() as u64).to_le_bytes());
    hasher.update(callback.unwrap_or("default").as_bytes());
    hasher.update((canonical.len() as u64).to_le_bytes());
    hasher.update(canonical.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn id_is_stable_and_distinct() {
        let a = Item::new(json!({"x": 1}), "https://e.com/a", "s", Some("d"));
        let b = Item::new(json!({"x": 1}), "https://e.com/a", "s", Some("d"));
        assert_eq!(a.id(), b.id());

        let c = Item::new(json!({"x": 2}), "https://e.com/a", "s", Some("d"));
        assert_ne!(a.id(), c.id());

        let d = Item::new(json!({"x": 1}), "https://e.com/b", "s", Some("d"));
        assert_ne!(a.id(), d.id());
    }

    #[test]
    fn with_value_recomputes_id() {
        let mut item = Item::new(json!({"x": 1}), "https://e.com/a", "s", None);
        let old = item.id().to_string();
        item.with_value(json!({"x": 2}));
        assert_ne!(item.id(), old);
        assert_eq!(item.value()["x"], 2);
    }

    #[test]
    fn try_typed_borrows_without_consuming() {
        let item = Item::new(json!({"title": "hi"}), "https://e.com/a", "s", None);
        #[derive(serde::Deserialize)]
        struct Novel {
            title: String,
        }
        let novel: Novel = item.try_typed().expect("typed");
        assert_eq!(novel.title, "hi");
        assert_eq!(item.value()["title"], "hi");
    }
}
