//! 小说爬虫性能剖析入口：本地小说站 + 声明式多 Spider 流程。
//!
//! 与 `novel_crawler` 使用同一套页面流程模板，但目标为本地服务器，
//! 排除真实站点网络波动，便于对框架 CPU/吞吐做可复现 profiling。
//!
//! 环境变量：
//! - `NOVEL_BOOKS`：首页书籍数（默认 30）
//! - `NOVEL_CHAPTERS`：每本书章节数（默认 50）
//! - `NOVEL_CHAPTER_KB`：章节页大小 KB（默认 8）
//! - `NOVEL_LOOPS`：整轮爬取次数（默认 10）
//! - `NOVEL_CONCURRENCY`：max_concurrent（默认 4）

use serde_json::json;
use std::time::{Duration, Instant};
use wisp::crawl::middleware::UaRotationMiddleware;
use wisp::crawl::stop::MaxPages;
use wisp::crawl::{ClosureSpider, Engine, SpiderBuilder};

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn novel_home_page(books: usize) -> std::collections::HashMap<String, Vec<u8>> {
    let links: String = (1..=books)
        .map(|i| {
            format!(r#"<div class="bookbox"><a class="s2" href="/book/{i}">Book {i}</a></div>"#)
        })
        .collect();
    let mut pages = std::collections::HashMap::new();
    pages.insert(
        "/".to_string(),
        format!("<html><head><title>Novel Home</title></head><body>{links}</body></html>")
            .into_bytes(),
    );
    pages
}

fn novel_book_pages(books: usize, chapters: usize) -> std::collections::HashMap<String, Vec<u8>> {
    let mut pages = std::collections::HashMap::new();
    for book in 1..=books {
        let links: String = (1..=chapters)
            .map(|c| format!(r#"<li class="name"><a class="name" href="/chapter/{book}/{c}">第{c}章</a></li>"#))
            .collect();
        pages.insert(
            format!("/book/{book}"),
            format!("<html><body><div class=\"list\"><ul>{links}</ul></div></body></html>")
                .into_bytes(),
        );
    }
    pages
}

fn novel_chapter_pages(
    books: usize,
    chapters: usize,
    chapter_kb: usize,
) -> std::collections::HashMap<String, Vec<u8>> {
    let paragraph = "这是用于性能剖析的章节正文。每一段都包含足够多的中文文本，用于衡量 HTML 解析、文本提取与内容清洗的开销。";
    let mut pages = std::collections::HashMap::new();
    for book in 1..=books {
        for c in 1..=chapters {
            let mut content = String::with_capacity(chapter_kb * 1024);
            while content.len() < chapter_kb * 1024 {
                content.push_str(paragraph);
                content.push('\n');
            }
            pages.insert(
                format!("/chapter/{book}/{c}"),
                format!("<html><body><div id=\"content\">{content}</div></body></html>")
                    .into_bytes(),
            );
        }
    }
    pages
}

fn build_novel_pages(
    books: usize,
    chapters: usize,
    chapter_kb: usize,
) -> std::sync::Arc<std::collections::HashMap<String, std::sync::Arc<Vec<u8>>>> {
    let mut pages = novel_home_page(books);
    pages.extend(novel_book_pages(books, chapters));
    pages.extend(novel_chapter_pages(books, chapters, chapter_kb));
    std::sync::Arc::new(
        pages
            .into_iter()
            .map(|(k, v)| (k, std::sync::Arc::new(v)))
            .collect(),
    )
}

async fn serve_novel_requests(
    listener: tokio::net::TcpListener,
    pages: std::sync::Arc<std::collections::HashMap<String, std::sync::Arc<Vec<u8>>>>,
) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let pages = std::sync::Arc::clone(&pages);
        tokio::spawn(async move {
            let mut reader = BufReader::new(socket);
            let mut line = Vec::with_capacity(512);
            loop {
                line.clear();
                loop {
                    let Ok(n) = reader.read_until(b'\n', &mut line).await else {
                        return;
                    };
                    if n == 0 || line.len() > 65536 {
                        return;
                    }
                    if line.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&line);
                let path = request
                    .lines()
                    .next()
                    .unwrap_or("GET / HTTP/1.1")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/");
                let body = pages.get(path).cloned().unwrap_or_default();
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                    body.len()
                );
                if reader.get_mut().write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                if reader.get_mut().write_all(&body).await.is_err() {
                    return;
                }
            }
        });
    }
}

async fn spawn_novel_server(books: usize, chapters: usize, chapter_kb: usize) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let pages = build_novel_pages(books, chapters, chapter_kb);
    tokio::spawn(serve_novel_requests(listener, pages));
    format!("http://{}", addr)
}

fn clean_content(text: String) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn novel_spiders(base: &str, book_limit: usize) -> Vec<ClosureSpider> {
    vec![
        SpiderBuilder::new("home")
            .start_urls(vec![format!("{base}/")])
            .on_links(
                "default",
                &["a.s2"],
                "detail",
                |_page, _idx, a| json!({ "title": a.text().trim() }),
            )
            .build(),
        SpiderBuilder::new("detail")
            .on_links("detail", &["a.name"], "chapter", |page, idx, a| {
                json!({
                    "title": page.meta_str("title"),
                    "author": page.select_one(".txt ul:nth-child(1)")
                        .map(|n| n.text().trim().to_string())
                        .unwrap_or_default(),
                    "chapter_title": a.text().trim(),
                    "chapter_index": idx,
                })
            })
            .until(MaxPages(book_limit))
            .build(),
        SpiderBuilder::new("chapter")
            .on_content("chapter", &["#content"], |text| async move {
                clean_content(text)
            })
            .build(),
    ]
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let books = env_usize("NOVEL_BOOKS", 30);
    let chapters = env_usize("NOVEL_CHAPTERS", 50);
    let chapter_kb = env_usize("NOVEL_CHAPTER_KB", 8);
    let loops = env_usize("NOVEL_LOOPS", 10);
    let concurrency = env_usize("NOVEL_CONCURRENCY", 4);

    let base = spawn_novel_server(books, chapters, chapter_kb).await;
    let engine = Engine::infra()
        .max_concurrent(concurrency)
        .max_pages(usize::MAX)
        .download_delay(Duration::ZERO)
        .obey_robots(false)
        .ua_rotation(UaRotationMiddleware::desktop())
        .headers(vec![("Accept".into(), "text/html".into())])
        .cookie_challenge(true)
        .build()?;

    let start = Instant::now();
    let mut total_pages = 0usize;
    let mut total_items = 0usize;
    for i in 0..loops {
        let spiders = novel_spiders(&base, books);
        let (stats, items) = engine.run_many(spiders).await?;
        let pages: usize = stats.iter().map(|s| s.pages_crawled).sum();
        let items_count = items.len();
        total_pages += pages;
        total_items += items_count;
        println!("loop {i}: {pages} pages, {items_count} items");
    }
    let elapsed = start.elapsed();
    println!(
        "total: {loops} loops, {total_pages} pages, {total_items} items, {:.3}s, {:.1} pages/s, {:.2} ms/page",
        elapsed.as_secs_f64(),
        total_pages as f64 / elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / total_pages as f64
    );
    Ok(())
}
