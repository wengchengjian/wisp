//! CSS selector auto-generation.

use super::Node;

/// Generate a unique CSS selector for a node.
pub fn generate_css(node: &Node) -> String {
    let attrs = node.attrs();

    // Prefer ID
    if let Some(id) = attrs.get("id") {
        return format!("#{}", id);
    }

    // Use tag + class
    let tag = attrs.get("tag").map(|s| s.as_str()).unwrap_or("div");
    if let Some(class) = attrs.get("class") {
        let first_class = class.split_whitespace().next().unwrap_or("");
        if !first_class.is_empty() {
            return format!("{}.{}", tag, first_class);
        }
    }

    // Fallback: tag with attributes
    let mut selector = tag.to_string();
    for (key, value) in &attrs {
        if key != "tag" && key != "class" && key != "id" {
            selector.push_str(&format!("[{}=\"{}\"]", key, value));
            break; // Just one attribute for brevity
        }
    }
    selector
}
