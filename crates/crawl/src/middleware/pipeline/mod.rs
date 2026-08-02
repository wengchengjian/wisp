//! 内建 Item Pipeline 实现。

mod batch;
mod filter;
mod jsonl;
mod output;

pub use batch::BatchItemPipeline;
pub use filter::FilterFieldsPipeline;
pub use jsonl::JsonlWriterPipeline;
pub use output::OutputWriterPipeline;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::CrawlContext;
    use crate::middleware::ItemPipeline;
    use crate::runtime::output::OutputFormat;
    use serde_json::json;
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

    #[tokio::test]
    async fn output_pipeline_writes_markdown() {
        let dir = std::env::temp_dir().join("wisp_pipeline_md");
        let _ = std::fs::remove_dir_all(&dir);
        let p = OutputWriterPipeline::new(OutputFormat::Markdown, dir.to_str().unwrap());
        let c = ctx();
        p.open(&c).await;
        p.process_item(
            json!({"url": "https://example.com/a", "html": "<h1>Hi</h1>"}),
            &c,
        )
        .await;
        p.close(&c).await;
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
        warc.open(&c).await;
        warc.process_item(
            json!({"url": "https://example.com/a", "html": "<p>x</p>"}),
            &c,
        )
        .await;
        warc.close(&c).await;
        let warc_content = std::fs::read_to_string(&warc_path).unwrap();
        assert!(warc_content.starts_with("WARC/1.1"));
        let _ = std::fs::remove_file(&warc_path);

        let json_path = std::env::temp_dir().join("wisp_pipeline.json");
        let json = OutputWriterPipeline::new(OutputFormat::Json, json_path.to_str().unwrap());
        json.open(&c).await;
        json.process_item(json!({"a": 1}), &c).await;
        json.process_item(json!({"a": 2}), &c).await;
        json.close(&c).await;
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
                flushed.lock().await.extend(items);
            }
        });

        p.process_item(json!("a"), &c).await;
        p.process_item(json!("b"), &c).await;
        assert_eq!(flushed.lock().await.len(), 2, "满 batch_size 应立即 flush");
        p.process_item(json!("c"), &c).await;
        p.close(&c).await;
        assert_eq!(flushed.lock().await.len(), 3, "close 应 flush 剩余 item");
    }

    #[tokio::test]
    async fn jsonl_writer_pipeline_writes_lines() {
        let suffix = wisp_core::utils::random::rand_suffix();
        let path = std::env::temp_dir().join(format!("wisp_jsonl_pipeline_{suffix}.jsonl"));
        let p = JsonlWriterPipeline::new(path.to_str().unwrap());
        let c = ctx();
        p.open(&c).await;
        p.process_item(json!("line1"), &c).await;
        p.close(&c).await;

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.trim_end().lines().count(), 1);
        let _ = std::fs::remove_file(&path);
    }
}
