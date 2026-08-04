//! Page / on_page / on_links API 集成测试。

use serde_json::{Value, json};
use wisp::crawl::{Page, Spider, SpiderBuilder};
use wisp::{CrawlRequest, Response};

fn response_from_request(req: CrawlRequest, html: &str) -> Response {
    let mut resp = Response::from_http(
        200,
        req.url.clone(),
        Default::default(),
        html.as_bytes().to_vec(),
        "text/html; charset=utf-8".into(),
        req.request.clone(),
    );
    resp.request = req;
    resp
}

fn response(html: &str, callback: Option<&str>, meta: Value) -> Response {
    let mut req = CrawlRequest::get("https://example.com/page").with_meta(meta);
    if let Some(cb) = callback {
        req = req.with_callback(cb);
    }
    response_from_request(req, html)
}

#[test]
fn page_basic_flow() {
    let resp = response(
        r#"<html><body>
            <h1>Hello</h1>
            <a class="item" href="/a">A</a>
            <a class="item" href="/b">B</a>
        </body></html>"#,
        None,
        json!({}),
    );

    let mut page = Page::new(resp);
    assert_eq!(page.select_one("h1").unwrap().text(), "Hello");
    assert_eq!(page.css("a.item").len(), 2);

    page.item(json!({"title": page.meta_str("title")}));
    page.follow_links(&[".missing", ".item"], "detail", |_page, idx, a| {
        json!({
            "idx": idx,
            "label": a.text().trim(),
        })
    });

    assert_eq!(page.items().len(), 1);
    assert_eq!(page.follows().len(), 2);
    assert_eq!(page.follows()[0].callback.as_deref(), Some("detail"));
    assert_eq!(page.follows()[0].meta["idx"], 0);
    assert_eq!(page.follows()[0].meta["label"], "A");

    let (items, follows) = page.finish();
    assert_eq!(items.len(), 1);
    assert_eq!(follows.len(), 2);
}

#[test]
fn follow_links_n_only_takes_first_n() {
    let resp = response(
        r#"<html><body>
            <a class="book" href="/b1">Book 1</a>
            <a class="book" href="/b2">Book 2</a>
            <a class="book" href="/b3">Book 3</a>
        </body></html>"#,
        None,
        json!({}),
    );

    let mut page = Page::new(resp);
    page.follow_links_n(&[".book"], "detail", 2, |_page, idx, a| {
        json!({
            "idx": idx,
            "title": a.text().trim(),
        })
    });

    assert_eq!(page.follows().len(), 2);
    assert_eq!(page.follows()[0].url, "https://example.com/b1");
    assert_eq!(page.follows()[1].url, "https://example.com/b2");
    assert_eq!(page.follows()[1].meta["title"], "Book 2");
}

#[tokio::test]
async fn on_page_routes_and_passes_meta() {
    let spider = SpiderBuilder::new("pipeline")
        .start_urls(vec!["https://example.com/".to_string()])
        .on_page("default", |mut page| {
            let title = page.select_one("h1").map(|n| n.text()).unwrap_or_default();
            page.item(json!({"page": title}));
            page.follow_links(&["a"], "detail", |_page, _idx, a| {
                json!({
                    "title": a.text().trim(),
                })
            });
            page
        })
        .on_page("detail", |mut page| {
            page.item(json!({"title": page.meta_str("title")}));
            page
        })
        .build();

    let (items, follows) = spider
        .handle(response(
            r#"<html><body><h1>Home</h1><a href="/d">Book</a></body></html>"#,
            None,
            json!({}),
        ))
        .await;

    assert_eq!(items[0]["page"], "Home");
    assert_eq!(follows.len(), 1);
    assert_eq!(follows[0].callback.as_deref(), Some("detail"));

    let detail_req = follows[0].clone();
    let (items2, _) = spider
        .handle(response_from_request(
            detail_req,
            "<html><body><h1>Detail</h1></body></html>",
        ))
        .await;
    assert_eq!(items2[0]["title"], "Book");
}

#[tokio::test]
async fn on_links_preset_uses_first_nonempty_selector() {
    let spider = SpiderBuilder::new("links")
        .start_urls(vec!["https://example.com/".to_string()])
        .on_links(
            "default",
            &[".missing", ".item"],
            "detail",
            |_page, idx, a| json!({"idx": idx, "text": a.text().trim()}),
        )
        .build();

    let (items, follows) = spider
        .handle(response(
            r#"<html><body><a class="item" href="/a">A</a><a class="item" href="/b">B</a></body></html>"#,
            None,
            json!({}),
        ))
        .await;

    assert!(items.is_empty());
    assert_eq!(follows.len(), 2);
    assert_eq!(follows[0].callback.as_deref(), Some("detail"));
    assert_eq!(follows[0].meta["idx"], 0);
    assert_eq!(follows[1].meta["idx"], 1);
    assert_eq!(follows[1].meta["text"], "B");
}

#[test]
fn page_item_value_avoids_reserialization() {
    let resp = response("<html><body><p>x</p></body></html>", None, json!({}));
    let mut page = Page::new(resp);
    let item = json!({"content": "hello", "n": 1});
    page.item_value(item.clone());
    assert_eq!(page.items().len(), 1);
    assert_eq!(page.items()[0], item);
}

#[tokio::test]
async fn on_links_n_preset_limits_follows() {
    let spider = SpiderBuilder::new("links-n")
        .start_urls(vec!["https://example.com/".to_string()])
        .on_links_n(
            "default",
            &[".item"],
            "detail",
            1,
            |_page, idx, a| json!({"idx": idx, "text": a.text().trim()}),
        )
        .build();

    let (_, follows) = spider
        .handle(response(
            r#"<html><body><a class="item" href="/a">A</a><a class="item" href="/b">B</a></body></html>"#,
            None,
            json!({}),
        ))
        .await;
    assert_eq!(follows.len(), 1);
    assert_eq!(follows[0].url, "https://example.com/a");
}

#[tokio::test]
async fn on_content_preset_builds_item_from_meta() {
    let spider = SpiderBuilder::new("content")
        .on_content("chapter", &[".content"], |text| text.trim().to_string())
        .build();

    let req = CrawlRequest::get("https://example.com/ch/1")
        .with_callback("chapter")
        .with_meta(json!({ "title": "Book" }));
    let resp = response_from_request(
        req,
        r#"<html><body><div class="content"> hello world </div></body></html>"#,
    );
    let (items, follows) = spider.handle(resp).await;
    assert!(follows.is_empty());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Book");
    assert_eq!(items[0]["content"], "hello world");
    assert_eq!(items[0]["url"], "https://example.com/ch/1");
}
