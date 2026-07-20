//! Criterion benchmarks for wisp parser performance.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use wisp::parser::Node;

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
    group.bench_function("100KB", |b| b.iter(|| Node::from_html(black_box(&html_100k))));
    group.bench_function("1MB", |b| b.iter(|| Node::from_html(black_box(&html_1m))));
    group.finish();
}

fn bench_css_select(c: &mut Criterion) {
    let html = generate_html(100);
    let doc = Node::from_html(&html);

    let mut group = c.benchmark_group("css_select");
    group.bench_function("simple_tag", |b| b.iter(|| doc.select(black_box("div"))));
    group.bench_function("class", |b| b.iter(|| doc.select(black_box(".item"))));
    group.bench_function("nested", |b| b.iter(|| doc.select(black_box("div.item p.desc"))));
    group.bench_function("attribute", |b| b.iter(|| doc.select(black_box("[data-price]"))));
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

criterion_group!(benches, bench_parse, bench_css_select, bench_text_extraction, bench_nodelist_iter);
criterion_main!(benches);
