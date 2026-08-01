//! 字符解码逻辑。

use crate::encoding::{
    detect::{extract_charset, extract_meta_charset},
    labels::encoding_from_label,
};
use encoding_rs::UTF_8;
use std::borrow::Cow;

pub fn decode(body: &[u8], content_type: &str) -> String {
    decode_borrowed(body, content_type).into_owned()
}

/// 解码为可能借用原始 body 的字符串，UTF-8 页面可避免一次 String 复制。

fn decode_utf8_cow<'a>(body: &'a [u8]) -> Cow<'a, str> {
    match std::str::from_utf8(body) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => Cow::Owned(String::from_utf8_lossy(body).into_owned()),
    }
}

fn decode_with_bom<'a>(body: &'a [u8]) -> Option<Cow<'a, str>> {
    if !body.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return None;
    }
    Some(decode_utf8_cow(&body[3..]))
}

fn decode_with_charset<'a>(body: &'a [u8], content_type: &str) -> Option<Cow<'a, str>> {
    let charset = extract_charset(content_type)?;
    let encoding = encoding_from_label(&charset)?;
    if encoding == UTF_8 {
        return Some(decode_utf8_cow(body));
    }
    let (decoded, _, _) = encoding.decode(body);
    Some(Cow::Owned(decoded.into_owned()))
}

fn decode_with_meta_charset<'a>(body: &'a [u8]) -> Option<Cow<'a, str>> {
    let preview = String::from_utf8_lossy(&body[..body.len().min(2048)]);
    let meta_charset = extract_meta_charset(&preview)?;
    let encoding = encoding_from_label(&meta_charset)?;
    let (decoded, _, _) = encoding.decode(body);
    Some(Cow::Owned(decoded.into_owned()))
}
pub fn decode_borrowed<'a>(body: &'a [u8], content_type: &str) -> Cow<'a, str> {
    if let Some(decoded) = decode_with_bom(body) {
        return decoded;
    }
    if let Some(decoded) = decode_with_charset(body, content_type) {
        return decoded;
    }
    if let Ok(s) = std::str::from_utf8(body) {
        return Cow::Borrowed(s);
    }
    if let Some(decoded) = decode_with_meta_charset(body) {
        return decoded;
    }
    Cow::Owned(String::from_utf8_lossy(body).into_owned())
}
