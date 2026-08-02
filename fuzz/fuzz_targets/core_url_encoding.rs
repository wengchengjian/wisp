#![no_main]

use libfuzzer_sys::fuzz_target;
use wisp_core::encoding::{decode, decode_borrowed};
use wisp_core::utils::url::{resolve_href, sanitize_url, url_to_filename};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let content_type: String = text.chars().take(256).collect();

    let _ = decode(data, &content_type);
    let _ = decode_borrowed(data, &content_type);
    let _ = resolve_href(&text, &text);
    let _ = sanitize_url(&text);
    let _ = url_to_filename(&text, data.len());
});
