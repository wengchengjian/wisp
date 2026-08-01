//! Character encoding detection and decoding.

mod decode;
mod detect;
mod labels;

pub use decode::{decode, decode_borrowed};
#[cfg(test)]
pub(crate) use detect::{extract_charset, extract_meta_charset};
#[cfg(test)]
pub(crate) use labels::encoding_from_label;

#[cfg(test)]
mod tests {
    use super::*;

    // === BOM 检测 ===

    #[test]
    fn test_utf8_bom() {
        let body = [0xEF, 0xBB, 0xBF, b'H', b'e', b'l', b'l', b'o'];
        assert_eq!(decode(&body, ""), "Hello");
    }

    #[test]
    fn test_utf16_bom_fallback() {
        // UTF-16 BOM 存在但无有效 UTF-16 内容，回退到 lossy UTF-8
        let body = [0xFF, 0xFE, 0x48, 0x00];
        let result = decode(&body, "");
        assert!(!result.is_empty());
    }

    // === Content-Type charset 检测 ===

    #[test]
    fn test_content_type_utf8() {
        let body = "你好世界".as_bytes();
        assert_eq!(decode(body, "text/html; charset=utf-8"), "你好世界");
    }

    #[test]
    fn test_content_type_gbk() {
        // GBK 编码的 "你好"
        let body: &[u8] = &[0xC4, 0xE3, 0xBA, 0xC3];
        assert_eq!(decode(body, "text/html; charset=gbk"), "你好");
    }

    #[test]
    fn test_content_type_gb2312() {
        let body: &[u8] = &[0xC4, 0xE3, 0xBA, 0xC3];
        assert_eq!(decode(body, "text/html; charset=gb2312"), "你好");
    }

    #[test]
    fn test_content_type_big5() {
        // Big5 编码的 "你好"
        let body: &[u8] = &[0xA7, 0x41, 0xA6, 0x6E];
        assert_eq!(decode(body, "text/html; charset=big5"), "你好");
    }

    #[test]
    fn test_content_type_shift_jis() {
        // Shift_JIS 编码的 "こんにちは"
        let body: &[u8] = &[0x82, 0xB1, 0x82, 0xF1, 0x82, 0xC9, 0x82, 0xBF, 0x82, 0xCD];
        assert_eq!(decode(body, "text/html; charset=shift_jis"), "こんにちは");
    }

    #[test]
    fn test_content_type_euc_jp() {
        // EUC-JP 编码的 "こんにちは"
        let body: &[u8] = &[0xA4, 0xB3, 0xA4, 0xF3, 0xA4, 0xCB, 0xA4, 0xC1, 0xA4, 0xCF];
        assert_eq!(decode(body, "text/html; charset=euc-jp"), "こんにちは");
    }

    #[test]
    fn test_content_type_euc_kr() {
        // EUC-KR 编码的 "안녕하세요"
        let body: &[u8] = &[0xBE, 0xC8, 0xB3, 0xE7, 0xC7, 0xCF, 0xBC, 0xBC, 0xBF, 0xE4];
        assert_eq!(decode(body, "text/html; charset=euc-kr"), "안녕하세요");
    }

    #[test]
    fn test_content_type_windows_1251() {
        // Windows-1251 编码的 "Привет"
        let body: &[u8] = &[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2];
        assert_eq!(decode(body, "text/html; charset=windows-1251"), "Привет");
    }

    #[test]
    fn test_content_type_windows_1252() {
        // Windows-1252: 0xE9 = é
        let body: &[u8] = &[b'c', b'a', b'f', 0xE9];
        assert_eq!(decode(body, "text/html; charset=windows-1252"), "café");
    }

    #[test]
    fn test_content_type_charset_case_insensitive() {
        let body: &[u8] = &[0xC4, 0xE3, 0xBA, 0xC3];
        assert_eq!(decode(body, "text/html; CHARSET=GBK"), "你好");
    }

    // === UTF-8 直接解码 ===

    #[test]
    fn test_plain_utf8_no_charset() {
        let body = "Hello, 世界!".as_bytes();
        assert_eq!(decode(body, ""), "Hello, 世界!");
    }

    #[test]
    fn test_empty_body() {
        assert_eq!(decode(b"", ""), "");
    }

    // === Meta charset 检测 ===

    #[test]
    fn test_meta_charset_gbk() {
        // 非 UTF-8 字节 + HTML meta charset 声明
        let mut body = b"<html><head><meta charset=\"gbk\"></head><body>".to_vec();
        body.extend_from_slice(&[0xC4, 0xE3, 0xBA, 0xC3]); // "你好" in GBK
        body.extend_from_slice(b"</body></html>");
        let result = decode(&body, "");
        assert!(
            result.contains("你好"),
            "应通过 meta charset 解码 GBK，实际: {}",
            result
        );
    }

    #[test]
    fn test_meta_charset_utf8() {
        let body = b"<html><head><meta charset=\"utf-8\"></head><body>Hello</body></html>";
        let result = decode(body, "");
        assert!(result.contains("Hello"));
    }

    // === 回退行为 ===

    #[test]
    fn test_invalid_bytes_fallback_lossy() {
        // 无效 UTF-8 且无 charset 声明，回退到 lossy UTF-8
        let body: &[u8] = &[0xFF, 0xFE, 0x48, 0x65, 0x6C, 0x6C, 0x6F];
        let result = decode(body, "");
        assert!(result.contains("Hello") || result.contains("\u{fffd}"));
    }

    // === extract_charset 单元测试 ===

    #[test]
    fn test_extract_charset_basic() {
        assert_eq!(
            extract_charset("text/html; charset=utf-8"),
            Some("utf-8".to_string())
        );
        assert_eq!(
            extract_charset("text/html; charset=GBK"),
            Some("GBK".to_string())
        );
        assert_eq!(extract_charset("text/html"), None);
        assert_eq!(extract_charset(""), None);
    }

    #[test]
    fn test_extract_charset_with_quotes() {
        assert_eq!(
            extract_charset("text/html; charset=\"utf-8\""),
            Some("utf-8".to_string())
        );
    }

    // === extract_meta_charset 单元测试 ===

    #[test]
    fn test_extract_meta_charset_basic() {
        assert_eq!(
            extract_meta_charset("<meta charset=\"utf-8\">"),
            Some("utf-8".to_string())
        );
        assert_eq!(
            extract_meta_charset("<meta charset='gbk'>"),
            Some("gbk".to_string())
        );
        assert_eq!(
            extract_meta_charset("<html><body>No charset</body></html>"),
            None
        );
    }

    // === encoding_from_label 单元测试 ===

    #[test]
    fn test_encoding_from_label_aliases() {
        assert!(encoding_from_label("utf-8").is_some());
        assert!(encoding_from_label("utf8").is_some());
        assert!(encoding_from_label("gbk").is_some());
        assert!(encoding_from_label("gb2312").is_some());
        assert!(encoding_from_label("gb18030").is_some());
        assert!(encoding_from_label("big5").is_some());
        assert!(encoding_from_label("shift_jis").is_some());
        assert!(encoding_from_label("sjis").is_some());
        assert!(encoding_from_label("euc-jp").is_some());
        assert!(encoding_from_label("euc-kr").is_some());
        assert!(encoding_from_label("windows-1251").is_some());
        assert!(encoding_from_label("cp1251").is_some());
        assert!(encoding_from_label("latin1").is_some());
        assert!(encoding_from_label("iso-8859-1").is_some());
    }
}
