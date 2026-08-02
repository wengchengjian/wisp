//! 调试脚本：验证 cookie 挑战自动解决 + 选择器匹配。
use wisp::http::Client;
use wisp::parser::Node;

fn cookie_headers(cookies: &[String]) -> Vec<(String, String)> {
    if cookies.is_empty() {
        vec![]
    } else {
        vec![("Cookie".to_string(), cookies.join("; "))]
    }
}

fn print_selector_results(html: &str) {
    let doc = Node::from_html(html);
    let selector = ".txt-list li .s2 a";
    let links = doc.select(selector);
    println!("\n选择器 {:?} 匹配到 {} 个链接:", selector, links.len());
    for (i, a) in links.iter().take(10).enumerate() {
        let title = a.text().trim().to_string();
        let href = a.attr("href").unwrap_or_default();
        println!("  {}. {} -> {}", i + 1, title, href);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder()
        .proxy("http://127.0.0.1:7897")
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .build()?;

    let mut cookies: Vec<String> = Vec::new();
    let resp = loop {
        let headers = cookie_headers(&cookies);
        let r = client.get("https://www.qishuxia.com/", &headers).await?;
        if r.status == 403
            && let Some(sc) = r.headers.get("set-cookie")
        {
            let pair = sc.split(';').next().unwrap_or("").to_string();
            if !pair.is_empty() && cookies.len() < 3 {
                println!("[Cookie挑战] 获取 cookie #{}: {}", cookies.len() + 1, pair);
                cookies.push(pair);
                continue;
            }
        }
        break r;
    };

    println!("\n最终状态: {}", resp.status);
    let html = resp.text()?;
    println!("页面大小: {} bytes", html.len());
    print_selector_results(&html);
    Ok(())
}
