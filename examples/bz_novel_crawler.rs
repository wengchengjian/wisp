//! 小说网站爬虫：`https://www.bz555555555.com/`
//!
//! 爬取「最新小说」列表页（`/shuku/0-postdate-0-{page}.html`），逐本进入详情页收集章节，
//! 再抓取每章正文。三个 Spider 通过 callback 路由共享同一队列。
//!
//! 运行：
//! ```bash
//! # 爬取最新 50 页列表（默认）
//! cargo run --example bz_novel_crawler -- --pages 50
//!
//! # 测试：只爬 2 页列表
//! cargo run --example bz_novel_crawler -- --pages 2
//! ```
//!
//! 站点由 Cloudflare 防护，使用 Auto 模式：先走 HTTP+cookie 快速路径，仅在被拦截时
//! 自动升级 Stealth 隐形浏览器通过检测，通过后的 cookie 由后续 HTTP 请求复用。

use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use wisp::crawl::Item;
use wisp::crawl::middleware::OutputWriterPipeline;
use wisp::crawl::stop::MaxPages;
use wisp::{Engine, FetchClientConfig, FetchMode, OutputFormat, SpiderBuilder};

/// 站点基址。
const BASE: &str = "https://www.bz555555555.com";

/// 列表页选择器：最新小说条目（书名 + 链接）。
const BOOK_SELECTORS: [&str; 1] = ["li.column-2 .name"];
/// 详情页选择器：章节条目。
const CHAPTER_SELECTORS: [&str; 1] = ["ul.list a"];
/// 详情页分类选择器：面包屑第二个链接。
const CATEGORY_SELECTOR: &str = ".breadcrumb .bd a";
/// 正文选择器：章节正文节点。
const CONTENT_SELECTORS: [&str; 1] = [".page-content"];

/// 清理章节正文：去空白行、广告行。
async fn clean_content(text: String) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !l.contains("本章未完") && !l.contains("点击下一页"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 生成最新小说列表页 URL（page 从 1 开始）。
fn list_urls(pages: usize) -> Vec<String> {
    (1..=pages)
        .map(|p| format!("{BASE}/shuku/0-postdate-0-{p}.html"))
        .collect()
}

/// 解析命令行参数，取 `--pages N`（默认 50）。
fn pages_from_args() -> usize {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--pages")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
}

/// 构建三个 Spider：列表 → 详情 → 正文。
fn spiders(pages: usize) -> Vec<wisp::ClosureSpider> {
    vec![
        // 列表页 Spider：爬取最新小说列表页，提取书名并 follow 到详情页。
        SpiderBuilder::new("list")
            .start_urls(list_urls(pages))
            .on_links(
                "default",
                &BOOK_SELECTORS,
                "detail",
                |_page, _idx, a| json!({ "book_title": a.text().trim() }),
            )
            .until(MaxPages(pages))
            .build(),
        // 详情页 Spider：提取分类与章节链接，携带书名/分类/章节名 follow 到正文页。
        SpiderBuilder::new("detail")
            .on_links("detail", &CHAPTER_SELECTORS, "chapter", |page, _idx, a| {
                // 面包屑第二个链接为分类名，如「其他类别」。
                let category = page
                    .css(CATEGORY_SELECTOR)
                    .get(1)
                    .map(|n| n.text().trim().to_string())
                    .unwrap_or_default();
                json!({
                    "book_title": page.meta_str("book_title"),
                    "category": category,
                    "chapter_title": a.text().trim(),
                })
            })
            .build(),
        // 正文页 Spider：提取并清洗正文，产出 item。
        SpiderBuilder::new("chapter")
            .on_content("chapter", &CONTENT_SELECTORS, clean_content)
            .build(),
    ]
}

/// 构建 Auto 引擎（绕过 Cloudflare，仅在拦截时升级 Stealth）。
fn build_engine(pages: usize) -> Result<Engine, Box<dyn std::error::Error>> {
    let output = format!("bz_novel_{pages}p.jsonl");
    let transport = FetchClientConfig {
        // 升级到 Stealth 时必须真实渲染（headed + offscreen），否则 Turnstile iframe 不加载，
        // 与 `Fetcher::stealth()` 的默认行为保持一致。
        force_headed_offscreen: true,
        human_mode: true,
        challenge_timeout: Duration::from_secs(45),
        max_concurrent_pages: 4,
        ..Default::default()
    };
    Ok(Engine::infra()
        // Auto 模式：先 HTTP+cookie，被拦截时 StealthUpgrade 自动切 Stealth，
        // 通过后 cookie 复用，后续请求仍走 HTTP。
        .fetch_mode(FetchMode::Auto)
        .fetch_client_config(transport)
        .max_concurrent(4)
        // 全局硬上限：为列表页 + 详情页 + 正文页留足余量。
        .max_pages(pages * 200 + 10)
        .download_delay(Duration::from_millis(300))
        .obey_robots(false)
        .proxy("http://127.0.0.1:7897")
        .headers(vec![
            ("Accept".into(), "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8".into()),
            ("Accept-Language".into(), "zh-CN,zh;q=0.9,en;q=0.8".into()),
            ("Referer".into(), format!("{BASE}/")),
            ("Upgrade-Insecure-Requests".into(), "1".into()),
        ])
        // 本地保存到 books/<category>/<book>.txt（章节合并单文件）
        .pipeline(Arc::new(novel_store::NovelStorePipeline::new("books")))
        .pipeline(Arc::new(OutputWriterPipeline::new_append(
            OutputFormat::Jsonl,
            &output,
        )))
        .build()?)
}

fn print_item_summary(items: &[Item]) {
    let books: std::collections::HashSet<&str> = items
        .iter()
        .filter_map(|item| item.value()["book_title"].as_str())
        .collect();
    println!(
        "\n共采集 {} 个章节条目，覆盖 {} 本书",
        items.len(),
        books.len()
    );
    for item in items.iter().take(3) {
        println!(
            "\n--- {} | {} ---",
            item.value()["book_title"].as_str().unwrap_or(""),
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
    let pages = pages_from_args();
    let engine = build_engine(pages)?;
    println!("=== 开始爬取 bz555555555.com 最新 {pages} 页小说 ===\n");

    let (stats, items) = engine.run_many(spiders(pages)).await?;
    println!("\n=== 爬取完成 ===");
    for s in &stats {
        println!("{}", s.summary());
    }
    print_item_summary(&items);
    Ok(())
}

/// 小说本地保存管道：同一本书的所有章节合并写入一个文件。
///
/// 仅本示例使用，故内联在此而非进入库。每个 item 需包含 `category`、`book_title`、
/// `chapter_title`、`content` 字段；同一本书追加写入 `root/<category>/<book>.txt`，
/// 章节以 `========== 章节名 ==========` 分隔。
mod novel_store {
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use wisp::crawl::Item;
    use wisp::crawl::middleware::{CrawlContext, ItemPipeline};
    use wisp::error::{Result, WispError};

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
            writeln!(writer, "========== {chapter} ==========").map_err(WispError::Io)?;
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
}
