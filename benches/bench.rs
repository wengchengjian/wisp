//! Criterion benchmarks for wisp parser + crawl concurrency performance.

use std::sync::OnceLock;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use tokio::runtime::Runtime;
use tracing_subscriber::prelude::*;
use wisp::parser::Node;

mod timing_layer;
use timing_layer::TimingLayer;

static TIMING: OnceLock<TimingLayer> = OnceLock::new();

/// 获取全局 TimingLayer（注册 global subscriber，只设一次）。
/// process_request 通过 tokio::spawn 在 worker 线程执行，
/// thread-local subscriber 抓不到，必须用 global。
fn timing() -> &'static TimingLayer {
    TIMING.get_or_init(|| {
        let layer = TimingLayer::new();
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(layer.clone()),
        );
        layer
    })
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
use wisp::crawl::{Engine, Request, Response, Spider};

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

/// 最小 Spider：N 个 start_urls，parse 返回空（不 follow），用于测纯抓取吞吐。
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
    // bench 测纯抓取吞吐，关闭 robots 检查（single-flight 已优化 robots 性能，
    // 此处关闭以隔离测量引擎调度/连接池/中间件链的纯开销；
    // 临时改为 true 可验证 RobotsMiddleware 在 keep-alive 下的真实开销）
    fn obey_robots(&self) -> bool {
        false
    }
    async fn parse(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
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
            .build()
            .unwrap();
        timing.reset();
        group.bench_with_input(
            BenchmarkId::from_parameter(concurrent),
            &concurrent,
            |b, _| {
                b.iter(|| {
                    rt.block_on(async {
                        let spider = BenchSpider { urls: urls.clone() };
                        engine.run(spider).await.unwrap()
                    })
                })
            },
        );
        println!("engine_concurrent_fetch/{} - Stage Timing:", concurrent);
        timing.print_summary();
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
                        .push(Request::get(&format!("https://example.com/{}", i)))
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
                            s.push(Request::get(&format!("https://example.com/t{}/{}", t, i)))
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
);
criterion_main!(benches);
