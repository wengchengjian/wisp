#![no_main]

use libfuzzer_sys::fuzz_target;
use wisp_parser::Node;

fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    let selector: String = html.chars().take(128).collect();

    let doc = Node::from_html(&html);
    let _ = doc.select(&selector).text();
    let _ = doc.select(&selector).html();
    let _ = doc.select(&selector).attr("class");
    let _ = doc.select_all(&selector).len();
    let _ = doc.select_one(&selector);
    let _ = doc.text();
    let _ = doc.html();
    let _ = doc.outer_html();
    let _ = doc.tag();
    let _ = doc.attrs();

    let fragment = Node::from_fragment(&html);
    for fixed in ["div", "div.item", "a[href]", "p[data-x]"] {
        let _ = fragment.select(fixed).text();
        let _ = fragment.select(fixed).html();
        let _ = fragment.select_all(fixed).len();
    }
});
