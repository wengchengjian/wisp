//! DOM 元素操作：点击、填充、等待、文本提取。

use wisp_core::error::{BrowserError, Result, WispError};
use crate::page::Page;
use crate::page::do_evaluate as evaluate;

/// 点击匹配选择器的元素。
pub async fn click(page: &Page, selector: &str) -> Result<()> {
    let js = format!(
        r#"(() => {{ const el = document.querySelector({}); if (!el) throw new Error('Element not found: {}'); el.click(); return true; }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );
    evaluate(page, &js).await.map_err(|e| match e {
        WispError::Browser(BrowserError::EvalError(msg)) if msg.contains("Element not found") => {
            WispError::Browser(BrowserError::ElementNotFound { selector: selector.to_string() })
        }
        other => other,
    })?;
    Ok(())
}

/// 向匹配选择器的输入框填充文本（触发 input/change 事件）。
pub async fn fill(page: &Page, selector: &str, value: &str) -> Result<()> {
    let js = format!(
        r#"(() => {{ const el = document.querySelector({}); if (!el) throw new Error('Element not found: {}'); el.focus(); el.value = {}; el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }})); return true; }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'"),
        serde_json::to_string(value).unwrap()
    );
    evaluate(page, &js).await.map_err(|e| match e {
        WispError::Browser(BrowserError::EvalError(msg)) if msg.contains("Element not found") => {
            WispError::Browser(BrowserError::ElementNotFound { selector: selector.to_string() })
        }
        other => other,
    })?;
    Ok(())
}

/// 等待匹配选择器的元素出现（轮询检测，超时返回错误）。
pub async fn wait_for_selector(page: &Page, selector: &str, timeout_ms: u64) -> Result<()> {
    let js = format!(
        r#"(async () => {{ const deadline = Date.now() + {}; while (Date.now() < deadline) {{ if (document.querySelector({})) return true; await new Promise(r => setTimeout(r, 100)); }} throw new Error('Timeout waiting for: {}'); }})()"#,
        timeout_ms,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );
    evaluate(page, &js).await.map_err(|e| match e {
        WispError::Browser(BrowserError::EvalError(msg)) if msg.contains("Timeout") => {
            WispError::Timeout(format!("wait_for_selector: {selector}"))
        }
        other => other,
    })?;
    Ok(())
}

/// 获取匹配选择器元素的 textContent。
pub async fn text_content(page: &Page, selector: &str) -> Result<String> {
    let js = format!(
        r#"(() => {{ const el = document.querySelector({}); if (!el) throw new Error('Element not found: {}'); return el.textContent || ''; }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );
    let value = evaluate(page, &js).await.map_err(|e| match e {
        WispError::Browser(BrowserError::EvalError(msg)) if msg.contains("Element not found") => {
            WispError::Browser(BrowserError::ElementNotFound { selector: selector.to_string() })
        }
        other => other,
    })?;
    Ok(value.as_str().unwrap_or("").to_string())
}
