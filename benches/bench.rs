//! Criterion benchmarks for wisp parser + crawl concurrency performance.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use tokio::runtime::Runtime;
use wisp::parser::Node;

mod timing_layer;

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
async fn spawn_html_server(html: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
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

    let mut group = c.benchmark_group("engine_concurrent_fetch");
    group.sample_size(20);
    for &concurrent in &[1usize, 4, 16] {
        let engine = Engine::infra()
            .max_concurrent(concurrent)
            .max_pages(50)
            .build()
            .unwrap();
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
    config = Criterion::default()
        .warm_up_time(std::time::Duration::from_secs(1))
        .measurement_time(std::time::Duration::from_secs(2))
        .sample_size(30);
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
