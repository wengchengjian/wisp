//! 小说本地保存管道：同一本书的所有章节合并写入一个文件。

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::Item;
use crate::middleware::{CrawlContext, ItemPipeline};
use wisp_core::error::{Result, WispError};

/// 将小说章节合并保存为本地文本文件的管道。
///
/// 每个 item 需包含 `category`、`book_title`、`chapter_title`、`content` 字段。
/// 同一本书的所有章节依次追加写入 `root/<category>/<book>.txt`，章节之间以
/// `========== 章节名 ==========` 分隔。目录自动创建，路径中的非法字符
/// （`/ \ : * ? " < > |` 等）会被替换为 `_`。
///
/// 章节按到达顺序追加；若需严格按顺序，可配合持久化队列或按章节索引排序的前置管道。
pub struct NovelStorePipeline {
    root: PathBuf,
    /// book 相对路径 -> 文件缓冲写入器。全书写入需串行治理，用互斥锁保护。
    writers: Mutex<HashMap<String, std::io::BufWriter<std::fs::File>>>,
}

impl NovelStorePipeline {
    /// 创建小说保存管道，`root` 为顶层目录（如 `books`）。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            writers: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ItemPipeline for NovelStorePipeline {
    async fn open(&self, _ctx: &CrawlContext) -> Result<()> {
        std::fs::create_dir_all(&self.root).map_err(WispError::Io)?;
        Ok(())
    }

    async fn process_item(
        &self,
        item: Item<Value>,
        _ctx: &CrawlContext,
    ) -> Result<Option<Item<Value>>> {
        let v = item.value();
        let category = v["category"].as_str().unwrap_or("未分类");
        let book = v["book_title"].as_str().unwrap_or("未知小说");
        let chapter = v["chapter_title"].as_str().unwrap_or("未知章节");
        let content = v["content"].as_str().unwrap_or("");

        let rel_category = sanitize(category);
        let rel_book = sanitize(book);
        let dir = self.root.join(&rel_category);
        std::fs::create_dir_all(&dir).map_err(WispError::Io)?;

        // 同一本书共享一个写入器：root/<category>/<book>.txt
        let key = format!("{rel_category}/{rel_book}");
        let file = dir.join(format!("{rel_book}.txt"));

        let mut writers = self
            .writers
            .lock()
            .map_err(|_| WispError::Io(std::io::Error::other("novel writers poisoned")))?;
        let writer = writers.entry(key).or_insert_with(|| {
            let f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file)
                .expect("open novel file");
            std::io::BufWriter::new(f)
        });
        writeln!(writer, "========== {chapter} ==========")
            .map_err(WispError::Io)?;
        writeln!(writer, "{content}").map_err(WispError::Io)?;
        writeln!(writer).map_err(WispError::Io)?;
        writer.flush().map_err(WispError::Io)?;

        Ok(Some(item))
    }

    async fn close(&self, _ctx: &CrawlContext) -> Result<()> {
        let mut writers = self
            .writers
            .lock()
            .map_err(|_| WispError::Io(std::io::Error::other("novel writers poisoned")))?;
        for w in writers.values_mut() {
            w.flush().map_err(WispError::Io)?;
        }
        Ok(())
    }
}

/// 清理路径/文件名的非法字符。
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_illegal_chars() {
        assert_eq!(sanitize("a/b:c*d"), "a_b_c_d");
        assert_eq!(sanitize("x\\y?z"), "x_y_z");
        assert_eq!(sanitize("正常书名"), "正常书名");
    }

    #[tokio::test]
    async fn merges_chapters_of_same_book_into_one_file() {
        use serde_json::json;
        let root = std::env::temp_dir().join("wisp_novel_pipeline_merge");
        let _ = std::fs::remove_dir_all(&root);
        let p = NovelStorePipeline::new(&root);
        let ctx = CrawlContext {
            spider_name: "s".into(),
            fetch_mode: wisp_fetcher::FetchMode::Http,
            max_concurrent: 1,
            max_pages: 1,
            obey_robots: false,
            pages_crawled: 0,
            errors: 0,
        };
        p.open(&ctx).await.unwrap();

        for (ch, body) in [("第一章 陨落", "正文一"), ("第二章 突破", "正文二")] {
            let item = Item::new(
                json!({
                    "category": "玄幻",
                    "book_title": "斗破苍穹",
                    "chapter_title": ch,
                    "content": body,
                }),
                "https://e.com/c",
                "s",
                None,
            );
            p.process_item(item, &ctx).await.unwrap();
        }
        p.close(&ctx).await.unwrap();

        // 一本书一个文件，且包含两章内容
        let file = root.join("玄幻").join("斗破苍穹.txt");
        assert!(file.exists(), "应合并为单文件: {file:?}");
        let content = std::fs::read_to_string(&file).unwrap();
        assert!(content.contains("========== 第一章 陨落 =========="));
        assert!(content.contains("正文一"));
        assert!(content.contains("========== 第二章 突破 =========="));
        assert!(content.contains("正文二"));

        // 不同分类/书名应分开
        let file2 = root.join("都市").join("别的书.txt");
        assert!(!file2.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}