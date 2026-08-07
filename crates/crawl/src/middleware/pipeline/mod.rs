//! 内建 Item Pipeline 实现。

mod batch;
mod filter;
mod output;

pub use batch::BatchItemPipeline;
pub use filter::FilterFieldsPipeline;
pub use output::OutputWriterPipeline;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Item;
    use crate::middleware::CrawlContext;
    use crate::middleware::ItemPipeline;
    use crate::runtime::output::OutputFormat;
    use serde_json::json;
    use wisp_core::error::WispError;
    use wisp_fetcher::FetchMode;

    fn ctx() -> CrawlContext {
        CrawlContext {
            spider_name: "test".into(),
            fetch_mode: FetchMode::Http,
            max_concurrent: 1,
            max_pages: 1,
            obey_robots: false,
            pages_crawled: 0,
            errors: 0,
        }
    }

    fn item(value: serde_json::Value) -> Item {
        Item::new(value, "https://example.com/page", "test", None)
    }

    #[tokio::test]
    async fn output_pipeline_writes_markdown() {
        let dir = std::env::temp_dir().join("wisp_pipeline_md");
        let _ = std::fs::remove_dir_all(&dir);
        let p = OutputWriterPipeline::new(OutputFormat::Markdown, dir.to_str().unwrap());
        let c = ctx();
        p.open(&c).await.unwrap();
        p.process_item(item(json!({ "html": "<h1>Hi</h1>" })), &c)
            .await
            .unwrap();
        p.close(&c).await.unwrap();
        let files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(&files[0]).unwrap();
        assert!(content.contains("Hi"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn output_pipeline_writes_warc_and_json() {
        let c = ctx();
        let warc_path = std::env::temp_dir().join("wisp_pipeline.warc");
        let warc = OutputWriterPipeline::new(OutputFormat::Warc, warc_path.to_str().unwrap());
        warc.open(&c).await.unwrap();
        warc.process_item(item(json!({ "html": "<p>x</p>" })), &c)
            .await
            .unwrap();
        warc.close(&c).await.unwrap();
        let warc_content = std::fs::read_to_string(&warc_path).unwrap();
        assert!(warc_content.starts_with("WARC/1.1"));
        let _ = std::fs::remove_file(&warc_path);

        let json_path = std::env::temp_dir().join("wisp_pipeline.json");
        let json = OutputWriterPipeline::new(OutputFormat::Json, json_path.to_str().unwrap());
        json.open(&c).await.unwrap();
        json.process_item(item(json!({ "a": 1 })), &c)
            .await
            .unwrap();
        json.process_item(item(json!({ "a": 2 })), &c)
            .await
            .unwrap();
        json.close(&c).await.unwrap();
        let json_content = std::fs::read_to_string(&json_path).unwrap();
        assert!(json_content.starts_with("["));
        assert!(json_content.ends_with("]\n"));
        let _ = std::fs::remove_file(&json_path);
    }

    #[tokio::test]
    async fn batch_pipeline_flushes_at_capacity_and_close() {
        let c = ctx();
        let flushed = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let flushed_clone = flushed.clone();
        let p = BatchItemPipeline::new(2, move |items| {
            let flushed = flushed_clone.clone();
            async move {
                flushed
                    .lock()
                    .await
                    .extend(items.into_iter().map(|i| i.into_value()));
                Ok(())
            }
        });

        p.process_item(item(json!("a")), &c).await.unwrap();
        p.process_item(item(json!("b")), &c).await.unwrap();
        assert_eq!(flushed.lock().await.len(), 2, "满 batch_size 应立即 flush");
        p.process_item(item(json!("c")), &c).await.unwrap();
        p.close(&c).await.unwrap();
        assert_eq!(flushed.lock().await.len(), 3, "close 应 flush 剩余 item");
    }

    #[tokio::test]
    async fn batch_pipeline_propagates_flush_error() {
        let c = ctx();
        let p = BatchItemPipeline::new(1, async |_items| {
            Err(WispError::Io(std::io::Error::other("flush boom")))
        });
        let err = p.process_item(item(json!("x")), &c).await.unwrap_err();
        assert!(err.to_string().contains("flush boom"));
    }
}
