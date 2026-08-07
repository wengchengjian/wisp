//! Criterion benchmarks for wisp parser + crawl concurrency performance.

use std::sync::{Arc, OnceLock};

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use serde_json::json;
use tokio::runtime::Runtime;
use tracing_subscriber::prelude::*;
use wisp::crawl::middleware::UaRotationMiddleware;
use wisp::crawl::stop::MaxPages;
use wisp::crawl::{ClosureSpider, SpiderBuilder};
use wisp::fetcher::FetchMode;
use wisp::parser::Node;
use wisp::storage::{MemoryStore, Store};

mod timing_layer;
use timing_layer::TimingLayer;

static TIMING: OnceLock<Option<TimingLayer>> = OnceLock::new();

/// 获取全局 TimingLayer（注册 global subscriber，只设一次）。
/// process_request 通过 tokio::spawn 在 worker 线程执行，
/// thread-local subscriber 抓不到，必须用 global。
/// 仅在设置 `WISP_TIMING=1` 时启用，避免观测层本身污染基准数据。
fn timing() -> Option<&'static TimingLayer> {
    TIMING
        .get_or_init(|| {
            std::env::var_os("WISP_TIMING")?;
            let layer = TimingLayer::new();
            let _ = tracing::subscriber::set_global_default(
                tracing_subscriber::registry().with(layer.clone()),
            );
            Some(layer)
        })
        .as_ref()
}

// ============================ parser benchmarks ============================

fn generate_html(size_kb: usize) -> String {
    let mut html = String::with_capacity(size_kb * 1024);
    html.push_str("<html><body>");
    let item = r#"<div class="item" id="item-1"><h2>Title</h2><p class="desc">Description text here</p><a href="https://example.com">Link</a><span data-price="9.99">$9.99</span></div>"#;
    while html.len() < size_kb * 1024 {
        html.push_str(item);
    }
    html.push_str("</body></html>");
    html
}

fn bench_parse(c: &mut Criterion) {
    let html_10k = generate_html(10);
    let html_100k = generate_html(100);
    let html_1m = generate_html(1024);

    let mut group = c.benchmark_group("parse");
    group.bench_function("10KB", |b| b.iter(|| Node::from_html(black_box(&html_10k))));
    group.bench_function("100KB", |b| {
        b.iter(|| Node::from_html(black_box(&html_100k)))
    });
    group.bench_function("1MB", |b| b.iter(|| Node::from_html(black_box(&html_1m))));
    group.finish();
}

fn bench_css_select(c: &mut Criterion) {
    let html = generate_html(100);
    let doc = Node::from_html(&html);

    let mut group = c.benchmark_group("css_select");
    group.bench_function("simple_tag", |b| b.iter(|| doc.select(black_box("div"))));
    group.bench_function("class", |b| b.iter(|| doc.select(black_box(".item"))));
    group.bench_function("nested", |b| {
        b.iter(|| doc.select(black_box("div.item p.desc")))
    });
    group.bench_function("attribute", |b| {
        b.iter(|| doc.select(black_box("[data-price]")))
    });
    group.finish();
}

fn bench_text_extraction(c: &mut Criterion) {
    let html = generate_html(100);
    let doc = Node::from_html(&html);

    c.bench_function("text_extraction", |b| {
        b.iter(|| {
            let items = doc.select(black_box(".item"));
            let _texts: Vec<String> = items.text();
        })
    });
}

fn bench_nodelist_iter(c: &mut Criterion) {
    let html = generate_html(100);
    let doc = Node::from_html(&html);
    let items = doc.select(".item");

    c.bench_function("nodelist_iter", |b| {
        b.iter(|| {
            let mut count = 0;
            for node in items.iter() {
                let _ = node.text();
                count += 1;
            }
            count
        })
    });
}

// ============================ crawl concurrency benchmarks ============================

use async_trait::async_trait;
use serde_json::Value;
use wisp::crawl::scheduling::Scheduler;
use wisp::crawl::{CrawlRequest, Engine, Request, Response, Spider};

/// 返回固定 HTML 的本地 HTTP 服务器，返回 base URL（如 `http://127.0.0.1:PORT`）。
///
/// 支持 HTTP/1.1 keep-alive：每个连接循环处理多个请求，
/// 让 wreq 连接池能复用 TCP 连接，避免每请求重新握手。
async fn spawn_html_server(html: &'static str) -> String {
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
            html.len(),
            html
        );
        let resp_bytes: Arc<[u8]> = Arc::from(resp.into_bytes());
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            let resp_bytes = resp_bytes.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(socket);
                let mut line = Vec::with_capacity(256);
                loop {
                    line.clear();
                    // 读请求头直到空行（\r\n），GET 请求无 body
                    loop {
                        match reader.read_until(b'\n', &mut line).await {
                            Ok(0) => return, // EOF，客户端关闭连接
                            Ok(_) => {}
                            Err(_) => return,
                        }
                        if line.ends_with(b"\r\n") && line.len() == 2 {
                            break; // 空行，请求头结束
                        }
                        line.clear();
                    }
                    // 发响应（BufReader 透传 AsyncWrite 到内部 socket）
                    if reader.get_mut().write_all(&resp_bytes).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    format!("http://{}", addr)
}

const BENCH_HTML: &str = r#"<html><body><div class="item"><h2>Title</h2><p class="desc">content</p></div></body></html>"#;

/// 最小 Spider：N 个 start_urls，handle 返回空（不 follow），用于测纯抓取吞吐。
struct BenchSpider {
    urls: Vec<String>,
}

#[async_trait]
impl Spider for BenchSpider {
    fn name(&self) -> &str {
        "bench"
    }
    fn start_urls(&self) -> Vec<String> {
        self.urls.clone()
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<CrawlRequest>) {
        (vec![], vec![])
    }
}

/// 并发抓取吞吐：测不同 max_concurrent 下抓取 50 页的耗时。
/// 验证 Engine 的并发调度、连接池复用、中间件链开销。
fn bench_engine_concurrent_fetch(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let base = rt.block_on(spawn_html_server(BENCH_HTML));
    let urls: Vec<String> = (0..50).map(|i| format!("{}/p{}", base, i)).collect();

    let timing = timing();
    let mut group = c.benchmark_group("engine_concurrent_fetch");
    group.sample_size(20);
    for &concurrent in &[1usize, 4, 16] {
        let engine = Engine::infra()
            .max_concurrent(concurrent)
            .max_pages(50)
            .obey_robots(false)
            .build()
            .unwrap();
        group.bench_with_input(
            BenchmarkId::from_parameter(concurrent),
            &concurrent,
            |b, _| {
                b.iter(|| {
                    if let Some(t) = timing {
                        t.reset();
                    }
                    rt.block_on(async {
                        let spider = BenchSpider { urls: urls.clone() };
                        engine.run(spider).await.unwrap()
                    })
                })
            },
        );
        if let Some(t) = timing {
            println!("engine_concurrent_fetch/{} - Stage Timing:", concurrent);
            t.print_summary();
        }
    }
    group.finish();
}

/// Scheduler 单线程 push 吞吐：1000 次 push 的耗时（含去重 set 更新）。
fn bench_scheduler_push(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    c.bench_function("scheduler_push_1000", |b| {
        b.iter(|| {
            rt.block_on(async {
                let sched = Scheduler::new();
                for i in 0..1000 {
                    sched
                        .push(Request::get(&format!("https://example.com/{}", i)).into())
                        .await;
                }
            })
        })
    });
}

/// Scheduler 多任务并发 push 吞吐：4 任务各 push 250，验证 DashMap/去重的并发竞争。
fn bench_scheduler_concurrent_push(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    c.bench_function("scheduler_concurrent_push_4x250", |b| {
        b.iter(|| {
            rt.block_on(async {
                let sched = std::sync::Arc::new(Scheduler::new());
                let mut handles = vec![];
                for t in 0..4u32 {
                    let s = sched.clone();
                    handles.push(tokio::spawn(async move {
                        for i in 0..250 {
                            s.push(
                                Request::get(&format!("https://example.com/t{}/{}", t, i)).into(),
                            )
                            .await;
                        }
                    }));
                }
                for h in handles {
                    h.await.unwrap();
                }
            })
        })
    });
}

// ============================ novel flow benchmarks ============================

/// 本地小说站：首页列出 books 本书，详情页列出 chapters 章，章节页返回正文。
/// 保持 keep-alive，模拟真实小说的页面结构与声明式 Spider 流程。
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
    let paragraph = "这是用于性能基准测试的章节正文。每一段都包含足够多的中文文本，用于衡量 HTML 解析、文本提取与内容清洗的开销。";
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

/// 运行一轮小说流程，返回 item 数。`transport=true` 时带上 UA/headers/cookie 中间件。
async fn run_novel_once(
    base: &str,
    mode: FetchMode,
    cache: Option<Arc<dyn Store>>,
    transport: bool,
) -> usize {
    let spiders = novel_spiders(base, 10);
    let mut engine = Engine::infra()
        .fetch_mode(mode)
        .max_concurrent(4)
        .max_pages(500)
        .obey_robots(false);
    if transport {
        engine = engine
            .ua_rotation(UaRotationMiddleware::desktop())
            .headers(vec![("Accept".into(), "text/html".into())])
            .cookie_challenge(true);
    }
    if let Some(store) = cache {
        engine = engine.cache_store(store);
    }
    let (_, items) = engine.build().unwrap().run_many(spiders).await.unwrap();
    items.len()
}

/// 小说爬虫三段式流程吞吐：首页 → 详情 → 章节，覆盖声明式 handler + 多 Spider 路由。
fn bench_novel_flow(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let base = rt.block_on(spawn_novel_server(10, 30, 8));
    let timing = timing();
    let mut group = c.benchmark_group("novel_flow");
    group.sample_size(10);
    group.bench_function("multi_spider_10books", |b| {
        b.iter(|| {
            if let Some(t) = timing {
                t.reset();
            }
            rt.block_on(async {
                black_box(run_novel_once(&base, FetchMode::Auto, None, true).await);
            });
            if let Some(t) = timing {
                println!("novel_flow Stage Timing:");
                t.print_summary();
            }
        })
    });
    group.finish();
}

/// 配置对照：Auto / Http / 最小传输链 / 缓存回放，量化中间件与缓存开销。
fn bench_novel_flow_variants(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let base = rt.block_on(spawn_novel_server(10, 30, 8));
    let timing = timing();
    let mut group = c.benchmark_group("novel_flow_variants");
    group.sample_size(10);

    group.bench_function("auto_default", |b| {
        b.iter(|| {
            if let Some(t) = timing {
                t.reset();
            }
            let n = rt.block_on(run_novel_once(&base, FetchMode::Auto, None, true));
            if let Some(t) = timing {
                println!("auto_default Stage Timing:");
                t.print_summary();
            }
            n
        })
    });
    group.bench_function("http_with_transport", |b| {
        b.iter(|| {
            if let Some(t) = timing {
                t.reset();
            }
            let n = rt.block_on(run_novel_once(&base, FetchMode::Http, None, true));
            if let Some(t) = timing {
                println!("http_with_transport Stage Timing:");
                t.print_summary();
            }
            n
        })
    });
    group.bench_function("http_minimal", |b| {
        b.iter(|| {
            if let Some(t) = timing {
                t.reset();
            }
            let n = rt.block_on(run_novel_once(&base, FetchMode::Http, None, false));
            if let Some(t) = timing {
                println!("http_minimal Stage Timing:");
                t.print_summary();
            }
            n
        })
    });

    // 缓存回放：同一 Engine 复用 MemoryStore，衡量稳定态缓存命中吞吐。
    let cache_engine = Engine::infra()
        .fetch_mode(FetchMode::Http)
        .cache_store(Arc::new(MemoryStore::default()))
        .max_concurrent(4)
        .max_pages(500)
        .obey_robots(false)
        .build()
        .unwrap();
    group.bench_function("http_cached_replay", |b| {
        b.iter(|| {
            if let Some(t) = timing {
                t.reset();
            }
            let n = rt.block_on(async {
                let spiders = novel_spiders(&base, 10);
                let (_, items) = cache_engine.run_many(spiders).await.unwrap();
                black_box(items.len())
            });
            if let Some(t) = timing {
                println!("http_cached_replay Stage Timing:");
                t.print_summary();
            }
            n
        })
    });
    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default();
    targets =
        bench_parse,
        bench_css_select,
        bench_text_extraction,
        bench_nodelist_iter,
        bench_engine_concurrent_fetch,
        bench_scheduler_push,
        bench_scheduler_concurrent_push,
        bench_novel_flow,
        bench_novel_flow_variants,
);
criterion_main!(benches);
