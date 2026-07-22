//! 调试脚本：验证 cookie 挑战自动解决 + 选择器匹配。
use wisp::http::Client;
use wisp::parser::Node;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder()
        .proxy("http://127.0.0.1:7897")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .build()?;

    // 模拟引擎的 cookie 挑战解决流程
    let mut cookies: Vec<String> = Vec::new();
    let mut resp = loop {
        let headers: Vec<(String, String)> = if cookies.is_empty() {
            vec![]
        } else {
            vec![("Cookie".to_string(), cookies.join("; "))]
        };

        let r = if headers.is_empty() {
            client.get("https://www.qishuxia.com/").await?
        } else {
            client.get_with_headers("https://www.qishuxia.com/", &headers).await?
        };

        if r.status == 403 {
            if let Some(sc) = r.headers.get("set-cookie") {
                let pair = sc.split(';').next().unwrap_or("").to_string();
                if !pair.is_empty() && cookies.len() < 3 {
                    println!("[Cookie挑战] 获取 cookie #{}: {}", cookies.len() + 1, pair);
                    cookies.push(pair);
                    continue;
                }
            }
        }
        break r;
    };

    println!("\n最终状态: {}", resp.status);
    let html = resp.text()?;
    println!("页面大小: {} bytes", html.len());

    // 测试选择器
    let doc = Node::from_html(&html);
    let selector = ".txt-list li .s2 a";
    let links = doc.select(selector);
    println!("\n选择器 {:?} 匹配到 {} 个链接:", selector, links.len());
    for (i, a) in links.iter().take(10).enumerate() {
        let title = a.text().trim().to_string();
        let href = a.attr("href").unwrap_or_default();
        println!("  {}. {} -> {}", i + 1, title, href);
    }

    Ok(())
}
