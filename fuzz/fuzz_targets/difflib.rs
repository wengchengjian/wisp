#![no_main]

use libfuzzer_sys::fuzz_target;
use wisp_parser::difflib::SequenceMatcher;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let chars: Vec<char> = text.chars().collect();
    let mid = chars.len() / 2;
    let a = &chars[..mid];
    let b = &chars[mid..];

    let matcher = SequenceMatcher::new(a, b);
    let _ = matcher.ratio();
    let _ = matcher.find_longest_match(0, a.len(), 0, b.len());

    let without_autojunk = SequenceMatcher::new(a, b).without_autojunk();
    let _ = without_autojunk.ratio();
    let _ = without_autojunk.find_longest_match(0, a.len(), 0, b.len());
});
