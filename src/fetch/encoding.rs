//! Character encoding detection and decoding.

use encoding_rs::{UTF_8, GBK, BIG5, EUC_JP, EUC_KR, SHIFT_JIS, WINDOWS_1251, WINDOWS_1252};

/// Decode bytes to string using Content-Type header and BOM detection.
pub fn decode(body: &[u8], content_type: &str) -> String {
    // 1. Check BOM
    if body.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&body[3..]).to_string();
    }
    if body.starts_with(&[0xFF, 0xFE]) || body.starts_with(&[0xFE, 0xFF]) {
        // UTF-16 BOM - let encoding_rs handle via UTF-8 fallback
    }

    // 2. Check Content-Type charset
    let charset = extract_charset(content_type);
    if let Some(cs) = &charset {
        if let Some(encoding) = encoding_from_label(cs) {
            let (decoded, _, _) = encoding.decode(body);
            return decoded.to_string();
        }
    }

    // 3. Try UTF-8
    if let Ok(s) = std::str::from_utf8(body) {
        return s.to_string();
    }

    // 4. Check meta charset in HTML
    let preview = String::from_utf8_lossy(&body[..body.len().min(2048)]);
    if let Some(meta_charset) = extract_meta_charset(&preview) {
        if let Some(encoding) = encoding_from_label(&meta_charset) {
            let (decoded, _, _) = encoding.decode(body);
            return decoded.to_string();
        }
    }

    // 5. Fallback: lossy UTF-8
    String::from_utf8_lossy(body).to_string()
}

fn extract_charset(content_type: &str) -> Option<String> {
    let lower = content_type.to_lowercase();
    if let Some(idx) = lower.find("charset=") {
        let rest = &content_type[idx + 8..];
        let charset: String = rest.chars().take_while(|c| *c != ';' && *c != ' ' && *c != '"').collect();
        if !charset.is_empty() { return Some(charset); }
    }
    None
}

fn extract_meta_charset(html: &str) -> Option<String> {
    // Look for <meta charset="...">
    let lower = html.to_lowercase();
    if let Some(idx) = lower.find("charset=") {
        let rest = &html[idx + 8..];
        let charset: String = rest.chars()
            .skip_while(|c| *c == '"' || *c == '\'')
            .take_while(|c| *c != '"' && *c != '\'' && *c != '>' && *c != ' ' && *c != ';')
            .collect();
        if !charset.is_empty() { return Some(charset); }
    }
    None
}

fn encoding_from_label(label: &str) -> Option<&'static encoding_rs::Encoding> {
    let label = label.trim().to_lowercase();
    match label.as_str() {
        "utf-8" | "utf8" => Some(UTF_8),
        "gbk" | "gb2312" | "gb18030" => Some(GBK),
        "big5" => Some(BIG5),
        "euc-jp" => Some(EUC_JP),
        "shift_jis" | "shift-jis" | "sjis" => Some(SHIFT_JIS),
        "euc-kr" => Some(EUC_KR),
        "windows-1251" | "cp1251" => Some(WINDOWS_1251),
        "windows-1252" | "cp1252" | "latin1" | "iso-8859-1" => Some(WINDOWS_1252),
        _ => encoding_rs::Encoding::for_label(label.as_bytes()),
    }
}
