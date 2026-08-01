//! 请求 meta 读取。

use super::*;
use serde_json::Value;

impl Page {
    /// 请求 meta。
    pub fn meta(&self) -> &Value {
        &self.resp.request.meta
    }

    /// 取出请求 meta（避免内容页把整份 meta 克隆进 item）。
    pub fn meta_owned(&mut self) -> Value {
        std::mem::take(&mut self.resp.request.meta)
    }

    /// 读取 meta 字符串字段，缺失时返回空字符串。
    pub fn meta_str(&self, key: &str) -> String {
        self.meta_str_or(key, "")
    }

    /// 读取 meta 字符串字段，缺失时使用默认值。
    pub fn meta_str_or(&self, key: &str, default: &str) -> String {
        self.meta()
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_string()
    }

    /// 读取 meta 数字字段，缺失时返回 0。
    pub fn meta_u64(&self, key: &str) -> u64 {
        self.meta().get(key).and_then(Value::as_u64).unwrap_or(0)
    }
}
