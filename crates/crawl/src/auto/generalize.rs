//! URL 泛化算法：把具体 URL 转成可复用的正则模板。

/// 将具体 URL 泛化为正则模板。
///
/// # 示例
/// - `/products/123` → `/products/\d+`
/// - `/user/deadbeef-cafe-1234/posts` → `/user/[a-f0-9-]+/posts`
/// - `/page/2` → `/page/\d+`
/// - `/about` → `/about`（不变）
/// - `/shuku/0-lastupdate-0-1.html` → `/shuku/\d+-lastupdate-\d+-\d+\.html`
pub fn generalize_url(url: &str) -> String {
    let path = url::Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| url.to_string());

    let segments: Vec<String> = path
        .split('/')
        .map(|seg| {
            if seg.is_empty() {
                return String::new();
            }
            // 纯数字 → \d+
            if seg.chars().all(|c| c.is_ascii_digit()) {
                return r"\d+".to_string();
            }
            // UUID/哈希 → [a-f0-9-]+
            if is_uuid_or_hash(seg) {
                return r"[a-f0-9-]+".to_string();
            }
            // 混合段（含数字的字母数字段）：把连续数字序列替换为 \d+
            // 例：0-lastupdate-0-1.html → \d+-lastupdate-\d+-\d+\.html
            if seg.chars().any(|c| c.is_ascii_digit()) {
                return generalize_mixed_segment(seg);
            }
            // 保留字面量（转义正则特殊字符）
            regex::escape(seg)
        })
        .collect();

    segments.join("/")
}

/// 泛化混合段：把所有连续数字序列替换为 `\d+`，其余字符转义。
///
/// 例：`0-lastupdate-0-1.html` → `\d+-lastupdate-\d+-\d+\.html`
///     `product123` → `product\d+`
fn generalize_mixed_segment(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            // 跳过后续连续数字
            while chars.peek().map_or(false, |c| c.is_ascii_digit()) {
                chars.next();
            }
            result.push_str(r"\d+");
        } else {
            // 转义正则特殊字符
            if "\\.+*?()|[]{}^$".contains(c) {
                result.push('\\');
            }
            result.push(c);
        }
    }
    result
}

/// 判断字符串是否像 UUID 或哈希值。
fn is_uuid_or_hash(s: &str) -> bool {
    s.len() >= 8
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && s.contains(|c: char| c.is_ascii_digit())
}
