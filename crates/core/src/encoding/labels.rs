//! 编码别名映射。

use encoding_rs::{BIG5, EUC_JP, EUC_KR, GBK, SHIFT_JIS, UTF_8, WINDOWS_1251, WINDOWS_1252};

fn utf_aliases(label: &str) -> Option<&'static encoding_rs::Encoding> {
    match label {
        "utf-8" | "utf8" => Some(UTF_8),
        _ => None,
    }
}

fn east_asian_aliases(label: &str) -> Option<&'static encoding_rs::Encoding> {
    match label {
        "gbk" | "gb2312" | "gb18030" => Some(GBK),
        "big5" => Some(BIG5),
        "euc-jp" => Some(EUC_JP),
        "shift_jis" | "shift-jis" | "sjis" => Some(SHIFT_JIS),
        "euc-kr" => Some(EUC_KR),
        _ => None,
    }
}

fn western_aliases(label: &str) -> Option<&'static encoding_rs::Encoding> {
    match label {
        "windows-1251" | "cp1251" => Some(WINDOWS_1251),
        "windows-1252" | "cp1252" | "latin1" | "iso-8859-1" => Some(WINDOWS_1252),
        _ => None,
    }
}

fn known_alias_encoding(label: &str) -> Option<&'static encoding_rs::Encoding> {
    utf_aliases(label)
        .or_else(|| east_asian_aliases(label))
        .or_else(|| western_aliases(label))
}

pub(crate) fn encoding_from_label(label: &str) -> Option<&'static encoding_rs::Encoding> {
    let label = label.trim().to_lowercase();
    known_alias_encoding(&label).or_else(|| encoding_rs::Encoding::for_label(label.as_bytes()))
}
