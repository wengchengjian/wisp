//! charset 检测。

pub(crate) fn extract_charset(content_type: &str) -> Option<String> {
    let lower = content_type.to_lowercase();
    if let Some(idx) = lower.find("charset=") {
        let rest = &content_type[idx + 8..];
        // 跳过可能的引号
        let rest = rest.trim_start_matches(['"', '\'']);
        let charset: String = rest
            .chars()
            .take_while(|c| *c != ';' && *c != ' ' && *c != '"' && *c != '\'')
            .collect();
        if !charset.is_empty() {
            return Some(charset);
        }
    }
    None
}

pub(crate) fn extract_meta_charset(html: &str) -> Option<String> {
    // Look for <meta charset="...">
    let lower = html.to_lowercase();
    if let Some(idx) = lower.find("charset=") {
        let rest = &html[idx + 8..];
        let charset: String = rest
            .chars()
            .skip_while(|c| *c == '"' || *c == '\'')
            .take_while(|c| *c != '"' && *c != '\'' && *c != '>' && *c != ' ' && *c != ';')
            .collect();
        if !charset.is_empty() {
            return Some(charset);
        }
    }
    None
}
