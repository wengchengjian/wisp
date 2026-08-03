//! 小说爬虫示例：同一爬虫以“多 handler”和“多 spider”两种方式实现。
//!
//! 运行：
//! `cargo run --example novel_crawler -- --mode handler`
//! `cargo run --example novel_crawler -- --mode spider`
//!
//! 两种模式都只爬取 10 本书。

use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use wisp::crawl::middleware::{JsonlWriterPipeline, UaRotationMiddleware};
use wisp::crawl::stop::MaxPages;
use wisp::crawl::{ClosureSpider, Engine, Item, SpiderBuilder};
/// 清理章节正文：去广告行、空行。
fn clean_content(text: String) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !l.contains("本章未完") && !l.contains("点击下一页"))
        .collect::<Vec<_>>()
        .join("\n")
}

const HOME_SELECTORS: [&str; 7] = [
    ".txt-list li .s2 a",
    ".list2 ul li .name a",
    ".listmain dl dd a",
    "#list dd a",
    ".bookbox .bookname a",
    ".book-item .title a",
    ".novellist li a",
];

const CHAPTER_SELECTORS: [&str; 3] = [
    "#section-list a",
    ".list ul li .name a",
    ".listmain dl dd a",
];

const CONTENT_SELECTORS: [&str; 6] = [
    "#content",
    "#chaptercontent",
    ".content",
    ".chapter-content",
    "#BookText",
    ".read-content",
];

fn handler_spider() -> ClosureSpider {
    SpiderBuilder::new("novel-handler")
        .start_urls(vec!["https://www.qishuxia.com/".to_string()])
        .on_links_n(
            "default",
            &HOME_SELECTORS,
            "detail",
            10,
            |_page, _idx, a| {
                json!({
                    "title": a.text().trim(),
                    "author": "",
                })
            },
        )
        .on_links("detail", &CHAPTER_SELECTORS, "chapter", |page, idx, a| {
            json!({
                "title": page.meta_str("title"),
                "author": page.select_one(".txt ul:nth-child(1)")
                    .map(|n| n.text().trim().to_string())
                    .unwrap_or_default(),
                "chapter_title": a.text().trim(),
                "chapter_index": idx,
            })
        })
        .on_content("chapter", &CONTENT_SELECTORS, clean_content)
        .until(MaxPages(100))
        .build()
}

fn multi_spiders() -> Vec<ClosureSpider> {
    vec![
        SpiderBuilder::new("home")
            .start_urls(vec!["https://www.qishuxia.com/".to_string()])
            .on_links("default", &HOME_SELECTORS, "detail", |_page, _idx, a| {
                json!({
                    "title": a.text().trim(),
                    "author": "",
                })
            })
            .build(),
        SpiderBuilder::new("detail")
            .on_links("detail", &CHAPTER_SELECTORS, "chapter", |page, idx, a| {
                json!({
                    "title": page.meta_str("title"),
                    "author": page.select_one(".txt ul:nth-child(1)")
                        .map(|n| n.text().trim().to_string())
                        .unwrap_or_default(),
                    "chapter_title": a.text().trim(),
                    "chapter_index": idx,
                })
            })
            .until(MaxPages(10))
            .build(),
        SpiderBuilder::new("chapter")
            .on_content("chapter", &CONTENT_SELECTORS, clean_content)
            .build(),
    ]
}

fn parse_mode() -> String {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "--mode".to_string());
    if mode == "--mode" {
        std::env::args()
            .nth(2)
            .unwrap_or_else(|| "handler".to_string())
    } else {
        mode
    }
}

fn output_path(mode: &str) -> &'static str {
    if mode == "spider" {
        "novel_spiders.jsonl"
    } else {
        "novel_handler.jsonl"
    }
}

fn build_engine(output: &str) -> Result<Engine, Box<dyn std::error::Error>> {
    Ok(Engine::infra()
        .max_concurrent(4)
        .max_pages(500)
        .download_delay(Duration::from_millis(500))
        .obey_robots(false)
        .proxy("http://127.0.0.1:7897")
        .headers(vec![
            ("Accept".into(), "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8".into()),
            ("Accept-Language".into(), "zh-CN,zh;q=0.9,en;q=0.8".into()),
            ("Referer".into(), "https://www.qishuxia.com/".into()),
            ("Connection".into(), "keep-alive".into()),
            ("Upgrade-Insecure-Requests".into(), "1".into()),
        ])
        .ua_rotation(UaRotationMiddleware::desktop())
        .cookie_challenge(true)
        .pipeline(Arc::new(JsonlWriterPipeline::new(output)))
        .build()?)
}

async fn run_spider_mode(engine: &Engine) -> Result<Vec<Item>, Box<dyn std::error::Error>> {
    let (stats, items) = engine.run_many(multi_spiders()).await?;
    println!("\n=== 爬取完成 ===");
    for s in &stats {
        println!("{}", s.summary());
    }
    Ok(items)
}

async fn run_handler_mode(engine: &Engine) -> Result<Vec<Item>, Box<dyn std::error::Error>> {
    let (stats, items) = engine.run(handler_spider()).await?;
    println!("\n=== 爬取完成 ===");
    println!("{}", stats.summary());
    Ok(items)
}

fn print_item_summary(items: &[Item]) {
    let books: std::collections::HashSet<&str> = items
        .iter()
        .filter_map(|item| item.value()["title"].as_str())
        .collect();
    println!(
        "共获取 {} 个章节条目，覆盖 {} 本书",
        items.len(),
        books.len()
    );
    for item in items.iter().take(3) {
        println!(
            "\n--- {} | {} ---",
            item.value()["title"].as_str().unwrap_or(""),
            item.value()["chapter_title"].as_str().unwrap_or("")
        );
        let content = item.value()["content"].as_str().unwrap_or("");
        let preview: String = content.chars().take(100).collect();
        println!("  内容预览: {}...", preview);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let mode = parse_mode();
    let output = output_path(&mode);
    let engine = build_engine(output)?;
    println!("=== 开始爬取 qishuxia.com ({mode} mode) ===\n");
    let items = if mode == "spider" {
        run_spider_mode(&engine).await?
    } else {
        run_handler_mode(&engine).await?
    };
    print_item_summary(&items);
    println!("\n结果已通过 pipeline 保存到 {output}");
    Ok(())
}
