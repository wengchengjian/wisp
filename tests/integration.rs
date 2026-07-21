use wisp::{Browser, LaunchOptions};

/// Helper: launch browser for tests. Returns None if no Chrome found.
async fn launch_test_browser() -> Option<Browser> {
    Browser::launch(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .await
    .ok()
}

#[tokio::test]
async fn test_navigator_webdriver_is_null() {
    let Some(browser) = launch_test_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let page = browser.new_page().await.unwrap();
    page.goto("about:blank").await.unwrap();

    let webdriver = page.evaluate("navigator.webdriver").await.unwrap();
    assert!(
        webdriver.is_null() || webdriver == serde_json::Value::Bool(false),
        "navigator.webdriver should be null or false, got: {webdriver}"
    );

    browser.close().await.unwrap();
}

#[tokio::test]
async fn test_evaluate_returns_value() {
    let Some(browser) = launch_test_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let page = browser.new_page().await.unwrap();
    page.goto("about:blank").await.unwrap();

    let result = page.evaluate("1 + 2").await.unwrap();
    assert_eq!(result, serde_json::json!(3));

    let result = page.evaluate("'hello' + ' ' + 'world'").await.unwrap();
    assert_eq!(result, serde_json::json!("hello world"));

    browser.close().await.unwrap();
}

#[tokio::test]
async fn test_navigation_and_title() {
    let Some(browser) = launch_test_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let page = browser.new_page().await.unwrap();
    page.goto("data:text/html,<title>Test Page</title><h1>Hello</h1>")
        .await
        .unwrap();

    let title = page.evaluate_as_string("document.title").await.unwrap();
    assert_eq!(title, "Test Page");

    browser.close().await.unwrap();
}

#[tokio::test]
async fn test_element_click_and_fill() {
    let Some(browser) = launch_test_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let page = browser.new_page().await.unwrap();
    page.goto("data:text/html,<input id='inp'><button id='btn' onclick='document.getElementById(\"inp\").value=\"clicked\"'>Go</button>")
        .await
        .unwrap();

    page.click("#btn").await.unwrap();
    let value = page.evaluate_as_string("document.getElementById('inp').value").await.unwrap();
    assert_eq!(value, "clicked");

    page.fill("#inp", "typed text").await.unwrap();
    let value = page.evaluate_as_string("document.getElementById('inp').value").await.unwrap();
    assert_eq!(value, "typed text");

    browser.close().await.unwrap();
}

#[tokio::test]
async fn test_screenshot_creates_file() {
    let Some(browser) = launch_test_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let page = browser.new_page().await.unwrap();
    page.goto("data:text/html,<h1>Screenshot Test</h1>")
        .await
        .unwrap();

    let path = std::env::temp_dir().join("patchright_test_screenshot.png");
    let path_str = path.to_str().unwrap();
    page.screenshot(path_str).await.unwrap();

    assert!(path.exists(), "Screenshot file should exist");
    let metadata = std::fs::metadata(&path).unwrap();
    assert!(metadata.len() > 0, "Screenshot should not be empty");

    let _ = std::fs::remove_file(&path);
    browser.close().await.unwrap();
}

/// Adaptive + crawl integration tests (no network required).
mod adaptive_test {
    use wisp::parser::Node;
    use wisp::storage::Store;

    const PRODUCT_HTML: &str = r#"
    <html><body>
      <div class="products">
        <div class="product" data-id="1">
          <h3 class="title">Widget</h3>
          <span class="price">$9.99</span>
        </div>
      </div>
    </body></html>
    "#;

    const PRODUCT_HTML_V2: &str = r#"
    <html><body>
      <section class="catalog">
        <article class="item" data-id="1">
          <h3 class="name">Widget</h3>
          <span class="cost">$9.99</span>
        </article>
      </section>
    </body></html>
    "#;

    #[test]
    fn test_end_to_end_adaptive_relocation() {
        let store = Store::open_in_memory().unwrap();
        let url = "https://shop.example.com/products";

        // Phase 1: capture snapshot
        let doc = Node::from_html(PRODUCT_HTML);
        let node = doc.css_adaptive(".title", "product-title", url, &store, true, 0.5);
        assert!(node.is_some());
        assert_eq!(node.unwrap().text(), "Widget");

        // Phase 2: site redesign, CSS fails, adaptive kicks in
        let doc2 = Node::from_html(PRODUCT_HTML_V2);
        let node2 = doc2.css_adaptive(".title", "product-title", url, &store, true, 0.5);
        assert!(node2.is_some(), "adaptive should relocate after redesign");
        assert_eq!(node2.unwrap().text(), "Widget");
    }
}
