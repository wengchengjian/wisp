//! Full example: fetch -> parse -> extract -> crawl.
//!
//! Demonstrates all wisp scraping modules working together.

use wisp::parser::Node;
use wisp::text::Text;

fn main() {
    // === 1. Parser: HTML parsing + CSS selectors ===
    println!("=== Parser Demo ===");
    let html = r#"
    <html>
    <body>
        <div class="products">
            <div class="product" data-id="1">
                <h2 class="name">Widget A</h2>
                <span class="price">$19.99</span>
                <a href="/products/1">Details</a>
            </div>
            <div class="product" data-id="2">
                <h2 class="name">Widget B</h2>
                <span class="price">$29.99</span>
                <a href="/products/2">Details</a>
            </div>
            <div class="product" data-id="3">
                <h2 class="name">Widget C</h2>
                <span class="price">$39.99</span>
                <a href="/products/3">Details</a>
            </div>
        </div>
        <footer>
            <p>Contact: sales@example.com | Visit https://example.com</p>
        </footer>
    </body>
    </html>"#;

    let doc = Node::from_html(html);

    // CSS selectors
    let products = doc.select(".product");
    println!("Found {} products", products.len());

    for product in products.iter() {
        let name = product.select_one(".name").map(|n| n.text()).unwrap_or_default();
        let price = product.select_one(".price").map(|n| n.text()).unwrap_or_default();
        let link = product.select_one("a").and_then(|a| a.attr("href")).unwrap_or_default();
        println!("  {} - {} ({})", name, price, link);
    }

    // Attribute access
    let first_id = doc.select_one(".product").and_then(|p| p.attr("data-id"));
    println!("First product ID: {:?}", first_id);

    // Selector generation
    if let Some(el) = doc.select_one(".price") {
        println!("Generated selector: {}", el.generate_selector());
    }

    // === 2. Text processing ===
    println!("\n=== Text Demo ===");
    let footer_text = doc.select_one("footer p").map(|n| n.text()).unwrap_or_default();
    let text = Text(&footer_text);
    println!("Emails: {:?}", text.extract_emails());
    println!("URLs: {:?}", text.extract_urls());
    println!("Clean: {}", text.clean());

    // === 3. Fetch (async - shown as reference) ===
    println!("\n=== Fetch (reference) ===");
    println!("  let client = Client::builder().timeout(d).build()?;");
    println!("  let resp = client.get(url).await?;");
    println!("  let doc = resp.parse()?;  // Directly get a Node!");

    // === 4. Crawl (async - shown as reference) ===
    println!("\n=== Crawl (reference) ===");
    println!("  struct MySpider;");
    println!("  impl Spider for MySpider {{ ... }}");
    println!("  let stats = Engine::new(MySpider).run().await?;");

    println!("\n=== Done! ===");
}
