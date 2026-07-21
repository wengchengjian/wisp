//! Items 集合与 JSONL 流式写入器。

use std::path::Path;
use serde_json::Value;
use crate::error::{WispError, Result};

/// 爬取结果集合
pub struct Items {
    items: Vec<Value>,
}

impl Items {
    pub fn new(items: Vec<Value>) -> Self { Self { items } }
    pub fn len(&self) -> usize { self.items.len() }
    pub fn is_empty(&self) -> bool { self.items.is_empty() }
    pub fn iter(&self) -> impl Iterator<Item = &Value> { self.items.iter() }

    /// 导出为 JSON 字符串（pretty）
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.items)
            .map_err(|e| WispError::Serialize(e.to_string()))
    }

    /// 导出为 JSONL（每行一个 JSON 对象）
    pub fn to_jsonl(&self) -> Result<String> {
        let mut out = String::new();
        for item in &self.items {
            let line = serde_json::to_string(item)
                .map_err(|e| WispError::Serialize(e.to_string()))?;
            out.push_str(&line);
            out.push('\n');
        }
        Ok(out)
    }

    /// 写入 JSON 文件
    pub fn to_json_file(&self, path: &Path) -> Result<()> {
        let json = self.to_json()?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// 写入 JSONL 文件
    pub fn to_jsonl_file(&self, path: &Path) -> Result<()> {
        let jsonl = self.to_jsonl()?;
        std::fs::write(path, jsonl)?;
        Ok(())
    }
}

/// 流式 JSONL 写入器（边爬边写，避免内存堆积）
pub struct JsonlWriter {
    file: std::fs::File,
}

impl JsonlWriter {
    pub fn new(path: &Path) -> Result<Self> {
        Ok(Self { file: std::fs::File::create(path)? })
    }

    pub fn write(&mut self, item: &Value) -> Result<()> {
        use std::io::Write;
        let line = serde_json::to_string(item)
            .map_err(|e| WispError::Serialize(e.to_string()))?;
        writeln!(self.file, "{}", line)?;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        use std::io::Write;
        self.file.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_items_to_json() {
        let items = Items::new(vec![json!({"a": 1}), json!({"b": 2})]);
        let s = items.to_json().unwrap();
        let parsed: Vec<Value> = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["a"], 1);
    }

    #[test]
    fn test_items_to_jsonl() {
        let items = Items::new(vec![json!({"a": 1}), json!({"b": 2})]);
        let s = items.to_jsonl().unwrap();
        let lines: Vec<&str> = s.trim_end().lines().collect();
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["a"], 1);
    }

    #[test]
    fn test_items_to_json_file() {
        let path = std::env::temp_dir().join("wisp_test_items.json");
        let items = Items::new(vec![json!({"x": 10})]);
        items.to_json_file(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: Vec<Value> = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed[0]["x"], 10);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_items_to_jsonl_file() {
        let path = std::env::temp_dir().join("wisp_test_items.jsonl");
        let items = Items::new(vec![json!({"x": 1}), json!({"x": 2}), json!({"x": 3})]);
        items.to_jsonl_file(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let count = content.trim_end().lines().count();
        assert_eq!(count, 3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_jsonl_writer_streaming() {
        let path = std::env::temp_dir().join("wisp_test_writer.jsonl");
        let mut writer = JsonlWriter::new(&path).unwrap();
        for i in 0..5 {
            writer.write(&json!({"i": i})).unwrap();
        }
        writer.flush().unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.trim_end().lines().collect();
        assert_eq!(lines.len(), 5);
        let last: Value = serde_json::from_str(lines[4]).unwrap();
        assert_eq!(last["i"], 4);
        let _ = std::fs::remove_file(&path);
    }
}
