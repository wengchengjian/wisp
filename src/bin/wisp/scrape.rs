pub(super) async fn run_scrape(
    url: String,
    selector: Option<String>,
    format: String,
) -> Result<(), Box<dyn std::error::Error>> {
    use wisp::http::Client;
    let client = Client::builder().build()?;
    let resp = client.get(&url, &[]).await?;
    let html = resp.text()?;
    if let Some(sel) = selector {
        use wisp::parser::Node;
        let doc = Node::from_html(&html);
        let nodes = doc.select(&sel);
        let items: Vec<serde_json::Value> = nodes
            .iter()
            .map(|n| serde_json::json!({ "text": n.text() }))
            .collect();
        match format.as_str() {
            "jsonl" => {
                for item in &items {
                    println!("{}", serde_json::to_string(item)?);
                }
            }
            _ => println!("{}", serde_json::to_string_pretty(&items)?),
        }
    } else {
        println!("{html}");
    }
    Ok(())
}
