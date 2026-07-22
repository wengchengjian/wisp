//! 统一 Fetcher 接口测试。
//!
//! 验证三模式（Http / Dynamic / Stealth）返回相同 Response 类型，
//! 且统一的解析 API（css/xpath/find_by_text/follow）正常工作。

use std::time::Duration;
use wisp::{Fetcher, FetchMode, Response, Request, Session};
use wisp::FetcherConfig;

// === 单元测试：Response 统一 API ===

fn html_response(html: &str) -> Response {
    Response::from_http(
        200,
        "https://quotes.toscrape.com/".to_string(),
        Default::default(),
        html.as_bytes().to_vec(),
        "text/html; charset=utf-8".to_string(),
        Some(Request::get("https://quotes.toscrape.com/")),
    )
}

#[test]
fn test_unified_response_css() {
    let resp = html_response(r#"
        <div class="quote"><span class="text">"Life is what happens..."</span><small class="author">John Lennon</small></div>
        <div class="quote"><span class="text">"The world as we have created it..."</span><small class="author">Albert Einstein</small></div>
    "#);

    let quotes = resp.css(".quote");
    assert_eq!(quotes.len(), 2);

    let texts = resp.css(".text");
    assert_eq!(texts.len(), 2);
    assert!(texts.text()[0].contains("Life is what happens"));
}

#[test]
fn test_unified_response_xpath() {
    let resp = html_response(r#"
        <ul><li class="item">A</li><li class="item">B</li><li class="item">C</li></ul>
    "#);

    let items = resp.xpath("//li[@class='item']");
    assert_eq!(items.len(), 3);
    assert_eq!(items.text(), vec!["A", "B", "C"]);
}

#[test]
fn test_unified_response_find_by_text() {
    let resp = html_response(r#"
        <small class="author">Albert Einstein</small>
        <small class="author">John Lennon</small>
        <small class="author">Einstein Smith</small>
    "#);

    // 精确匹配
    let exact = resp.find_by_text("Albert Einstein", Some("small"), true);
    assert_eq!(exact.len(), 1);

    // 模糊匹配
    let contains = resp.find_by_text("Einstein", Some("small"), false);
    assert_eq!(contains.len(), 2);
}

#[test]
fn test_unified_response_select_one() {
    let resp = html_response(r#"<h1 id="title">Quotes to Scrape</h1>"#);
    let node = resp.select_one("#title").unwrap();
    assert_eq!(node.text(), "Quotes to Scrape");
}

#[test]
fn test_unified_response_follow() {
    let resp = html_response("");

    // 相对链接
    let req = resp.follow("/page/2/").unwrap();
    assert_eq!(req.url, "https://quotes.toscrape.com/page/2/");

    // 绝对链接
    let req = resp.follow("https://other.com/").unwrap();
    assert_eq!(req.url, "https://other.com/");

    // 带 callback
    let req = resp.follow_with("/page/2/", "parse_page").unwrap();
    assert_eq!(req.callback, Some("parse_page".to_string()));
}

#[test]
fn test_unified_response_json() {
    let resp = Response::from_http(
        200,
        "https://api.example.com/data".to_string(),
        Default::default(),
        br#"{"quotes": [{"text": "hello", "author": "world"}]}"#.to_vec(),
        "application/json".to_string(),
        None,
    );

    let json = resp.json().unwrap();
    assert_eq!(json["quotes"][0]["text"], "hello");
}

#[test]
fn test_unified_response_parse_and_navigate() {
    let resp = html_response(r#"
        <div class="quote">
            <span class="text">"Quote 1"</span>
            <small class="author">Author 1</small>
        </div>
    "#);

    // parse() 返回 Node，也可以继续导航
    let doc = resp.parse();
    let quote = doc.select_one(".quote").unwrap();
    let text = quote.select_one(".text").unwrap();
    assert_eq!(text.text(), "\"Quote 1\"");

    // 验证 parent 导航
    let parent = text.parent().unwrap();
    assert_eq!(parent.attr("class"), Some("quote".to_string()));
}

// === Fetcher Builder 配置测试 ===

#[test]
fn test_fetcher_three_modes_return_same_type() {
    // 验证三模式的 builder 都能构建，且返回相同 Fetcher 类型
    let http = Fetcher::http().build();
    let dynamic = Fetcher::dynamic().build();
    let stealth = Fetcher::stealth().build();

    assert_eq!(http.mode(), FetchMode::Http);
    assert_eq!(dynamic.mode(), FetchMode::Dynamic);
    assert_eq!(stealth.mode(), FetchMode::Stealth);
}

#[test]
fn test_fetcher_builder_full_config() {
    let fetcher = Fetcher::stealth()
        .proxy("http://127.0.0.1:7897")
        .timeout(Duration::from_secs(60))
        .headless(true)
        .human_mode(true)
        .challenge_timeout(Duration::from_secs(45))
        .wait_for(".content")
        .extra_wait_ms(1000)
        .block_ads()
        .block_domains(&["analytics.google.com"])
        .dns_over_https("https://1.1.1.1/dns-query")
        .build();

    let config = fetcher.config();
    assert_eq!(config.proxy.as_deref(), Some("http://127.0.0.1:7897"));
    assert_eq!(config.timeout, Duration::from_secs(60));
    assert!(config.headless);
    assert!(config.human_mode);
    assert_eq!(config.challenge_timeout, Duration::from_secs(45));
    assert_eq!(config.wait_for.as_deref(), Some(".content"));
    assert_eq!(config.extra_wait_ms, 1000);
    assert!(config.domain_blocker.is_some());
    assert_eq!(config.dns_over_https.as_deref(), Some("https://1.1.1.1/dns-query"));
}

// === Session 测试 ===

#[test]
fn test_session_three_modes() {
    let http_session = Session::http().build();
    let dynamic_session = Session::dynamic().build();
    let stealth_session = Session::stealth().proxy("http://127.0.0.1:7897").build();

    assert!(http_session.is_ok());
    assert!(dynamic_session.is_ok());
    assert!(stealth_session.is_ok());

    assert_eq!(http_session.unwrap().fetcher().mode(), FetchMode::Http);
    assert_eq!(dynamic_session.unwrap().fetcher().mode(), FetchMode::Dynamic);
    assert_eq!(stealth_session.unwrap().fetcher().mode(), FetchMode::Stealth);
}

#[tokio::test]
async fn test_session_cookie_management() {
    let session = Session::http().build().unwrap();

    session.set_cookie("example.com", "token", "abc123").await;
    session.set_cookie("example.com", "sid", "xyz").await;

    let cookies = session.cookies_for("example.com").await;
    assert_eq!(cookies.get("token").unwrap(), "abc123");
    assert_eq!(cookies.get("sid").unwrap(), "xyz");

    session.clear_cookies().await;
    let cookies = session.cookies_for("example.com").await;
    assert!(cookies.is_empty());
}

// === 真实网络测试（需要网络）===

#[tokio::test]
#[ignore = "requires network access"]
async fn test_unified_http_fetch_real() {
    let resp = Fetcher::http()
        .timeout(Duration::from_secs(15))
        .get("https://quotes.toscrape.com/")
        .await;

    match resp {
        Ok(page) => {
            assert!(page.is_ok());
            assert_eq!(page.url, "https://quotes.toscrape.com/");

            // 统一解析 API
            let quotes = page.css(".quote");
            assert!(quotes.len() >= 5, "应至少 5 条名言");

            let authors = page.xpath("//small[@class='author']");
            assert!(authors.len() >= 5);

            // find_by_text
            let einstein = page.find_by_text("Albert Einstein", Some("small"), true);
            assert!(!einstein.is_empty());

            // follow
            let next = page.follow("/page/2/");
            assert!(next.is_some());
            assert_eq!(next.unwrap().url, "https://quotes.toscrape.com/page/2/");

            println!("PASS: 统一 HTTP 接口真实测试通过");
        }
        Err(e) => {
            eprintln!("SKIP: 网络不可达: {}", e);
        }
    }
}
