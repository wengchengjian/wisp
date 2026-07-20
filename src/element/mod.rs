use chromiumoxide::Page as CdpPage;

use crate::error::{PatchrightError, Result};
use crate::page::evaluate;

/// Click an element matching the CSS selector.
pub async fn click(page: &CdpPage, selector: &str) -> Result<()> {
    let js = format!(
        r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found: {}');
            el.click();
            return true;
        }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );

    evaluate::evaluate(page, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Element not found") => {
            PatchrightError::ElementNotFound { selector: selector.to_string() }
        }
        other => other,
    })?;

    Ok(())
}

/// Type text into an input element matching the CSS selector.
pub async fn fill(page: &CdpPage, selector: &str, value: &str) -> Result<()> {
    let js = format!(
        r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found: {}');
            el.focus();
            el.value = {};
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
            el.dispatchEvent(new Event('change', {{ bubbles: true }}));
            return true;
        }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'"),
        serde_json::to_string(value).unwrap()
    );

    evaluate::evaluate(page, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Element not found") => {
            PatchrightError::ElementNotFound { selector: selector.to_string() }
        }
        other => other,
    })?;

    Ok(())
}

/// Wait for an element matching the selector to appear in the DOM.
pub async fn wait_for_selector(page: &CdpPage, selector: &str, timeout_ms: u64) -> Result<()> {
    let js = format!(
        r#"(async () => {{
            const deadline = Date.now() + {};
            while (Date.now() < deadline) {{
                if (document.querySelector({})) return true;
                await new Promise(r => setTimeout(r, 100));
            }}
            throw new Error('Timeout waiting for: {}');
        }})()"#,
        timeout_ms,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );

    evaluate::evaluate(page, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Timeout") => {
            PatchrightError::Timeout(format!("wait_for_selector: {selector}"))
        }
        other => other,
    })?;

    Ok(())
}

/// Get the text content of an element.
pub async fn text_content(page: &CdpPage, selector: &str) -> Result<String> {
    let js = format!(
        r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found: {}');
            return el.textContent || '';
        }})()"#,
        serde_json::to_string(selector).unwrap(),
        selector.replace('\'', "\\'")
    );

    let value = evaluate::evaluate(page, &js).await.map_err(|e| match e {
        PatchrightError::EvalError(msg) if msg.contains("Element not found") => {
            PatchrightError::ElementNotFound { selector: selector.to_string() }
        }
        other => other,
    })?;

    Ok(value.as_str().unwrap_or("").to_string())
}
