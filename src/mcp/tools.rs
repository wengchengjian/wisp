//! MCP 工具实现。

use serde_json::{Value, json};
use std::sync::Arc;
use crate::error::{WispError, Result};
use crate::storage::Store;
use crate::parser::Node;
use crate::fetch::Client;
use wreq_util::Profile;

/// 抓取单个网页，返回 HTML 文本。
pub async fn fetch_page(args: Value) -> Result<Value> {
    let url = args.get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'url' argument".into()))?;

    let mut builder = Client::builder();
    if let Some(emu) = args.get("emulation").and_then(|v| v.as_str()) {
        // Profile 变体名查证：Firefox128/Safari18 已存在（Stage 3 验证），
        // 计划假设的 FirefoxLatest/SafariLatest 不存在，改用具体版本号。
        let profile = match emu {
            "firefox" => Profile::Firefox128,
            "safari" => Profile::Safari18,
            _ => Profile::Chrome136,
        };
        builder = builder.emulation(profile);
    }

    let client = builder.build()?;
    let resp = client.get(url).await?;
    let html = resp.text()?;

    Ok(json!({
        "url": url,
        "status": resp.status,
        "html": html,
        "bytes": resp.body.len()
    }))
}

/// CSS 选择器提取元素。
pub async fn extract_css(args: Value) -> Result<Value> {
    let html = args.get("html")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'html' argument".into()))?;
    let selector = args.get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'selector' argument".into()))?;
    let attr: Option<&str> = args.get("attr").and_then(|v| v.as_str());

    let doc = Node::from_html(html);
    let nodes = doc.select(selector);

    if let Some(a) = attr {
        let attrs: Vec<Value> = nodes.iter()
            .map(|n| json!(n.attr(a).unwrap_or_default()))
            .collect();
        Ok(json!({"attrs": attrs}))
    } else {
        let texts: Vec<Value> = nodes.iter()
            .map(|n| json!(n.text()))
            .collect();
        Ok(json!({"texts": texts}))
    }
}

/// XPath 提取元素。
pub async fn extract_xpath(args: Value) -> Result<Value> {
    let html = args.get("html")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'html' argument".into()))?;
    let xpath = args.get("xpath")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'xpath' argument".into()))?;

    let doc = Node::from_html(html);
    let nodes = doc.xpath(xpath);

    let texts: Vec<Value> = nodes.iter()
        .map(|n| json!(n.text()))
        .collect();
    Ok(json!({"texts": texts}))
}

pub async fn crawl_site(args: Value, _store: &Arc<Store>) -> Result<Value> {
    let _ = args;
    Err(crate::error::WispError::McpError("crawl_site not implemented yet".into()))
}

pub async fn adaptive_scrape(args: Value, _store: &Arc<Store>) -> Result<Value> {
    let _ = args;
    Err(crate::error::WispError::McpError("adaptive_scrape not implemented yet".into()))
}

pub async fn stealth_fetch(args: Value) -> Result<Value> {
    let _ = args;
    Err(crate::error::WispError::McpError("stealth_fetch not implemented yet".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_extract_css_returns_text() {
        let args = json!({
            "html": "<html><body><p class='x'>hello</p><p class='x'>world</p></body></html>",
            "selector": "p.x"
        });
        let result = extract_css(args).await.unwrap();
        let texts = result["texts"].as_array().unwrap();
        assert_eq!(texts.len(), 2);
        assert_eq!(texts[0].as_str().unwrap(), "hello");
        assert_eq!(texts[1].as_str().unwrap(), "world");
    }

    #[tokio::test]
    async fn test_extract_css_returns_attr() {
        let args = json!({
            "html": "<html><body><a href='/a'>A</a><a href='/b'>B</a></body></html>",
            "selector": "a",
            "attr": "href"
        });
        let result = extract_css(args).await.unwrap();
        let attrs = result["attrs"].as_array().unwrap();
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].as_str().unwrap(), "/a");
    }

    #[tokio::test]
    async fn test_extract_xpath_returns_text() {
        let args = json!({
            "html": "<html><body><ul><li>1</li><li>2</li></ul></body></html>",
            "xpath": "//li"
        });
        let result = extract_xpath(args).await.unwrap();
        let texts = result["texts"].as_array().unwrap();
        assert_eq!(texts.len(), 2);
        assert_eq!(texts[0].as_str().unwrap(), "1");
    }

    #[tokio::test]
    async fn test_extract_css_missing_args() {
        let args = json!({});
        let result = extract_css(args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_page_missing_url() {
        let args = json!({});
        let result = fetch_page(args).await;
        assert!(result.is_err());
    }
}
