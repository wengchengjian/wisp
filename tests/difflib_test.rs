//! Verify SequenceMatcher matches Python difflib's ratio() output.
//! Reference values produced by:
//!   python3 -c "import difflib; print(difflib.SequenceMatcher(None, A, B).ratio())"

use wisp::parser::difflib::SequenceMatcher;

#[test]
fn test_ratio_identical_strings() {
    // Python: difflib.SequenceMatcher(None, "abc", "abc").ratio() == 1.0
    let a: Vec<char> = "abc".chars().collect();
    let b: Vec<char> = "abc".chars().collect();
    let ratio = SequenceMatcher::new(&a, &b).ratio();
    assert!((ratio - 1.0).abs() < 1e-9, "expected 1.0, got {}", ratio);
}

#[test]
fn test_ratio_completely_different() {
    // Python: difflib.SequenceMatcher(None, "abc", "xyz").ratio() == 0.0
    let a: Vec<char> = "abc".chars().collect();
    let b: Vec<char> = "xyz".chars().collect();
    let ratio = SequenceMatcher::new(&a, &b).ratio();
    assert!(ratio.abs() < 1e-9, "expected 0.0, got {}", ratio);
}

#[test]
fn test_ratio_partial_overlap() {
    // Python: difflib.SequenceMatcher(None, "abcd", "abce").ratio() == 0.75
    let a: Vec<char> = "abcd".chars().collect();
    let b: Vec<char> = "abce".chars().collect();
    let ratio = SequenceMatcher::new(&a, &b).ratio();
    assert!((ratio - 0.75).abs() < 1e-9, "expected 0.75, got {}", ratio);
}

#[test]
fn test_ratio_empty_inputs() {
    // Python: difflib.SequenceMatcher(None, "", "").ratio() == 1.0
    let a: Vec<char> = Vec::new();
    let b: Vec<char> = Vec::new();
    let ratio = SequenceMatcher::new(&a, &b).ratio();
    assert!((ratio - 1.0).abs() < 1e-9, "expected 1.0 for empty inputs, got {}", ratio);
}

#[test]
fn test_ratio_one_empty() {
    // Python: difflib.SequenceMatcher(None, "abc", "").ratio() == 0.0
    let a: Vec<char> = "abc".chars().collect();
    let b: Vec<char> = Vec::new();
    let ratio = SequenceMatcher::new(&a, &b).ratio();
    assert!(ratio.abs() < 1e-9, "expected 0.0, got {}", ratio);
}

#[test]
fn test_ratio_word_sequence() {
    // Python: difflib.SequenceMatcher(None, ["a","b","c","d"], ["a","x","c","y"]).ratio() == 0.5
    let a = vec!["a", "b", "c", "d"];
    let b = vec!["a", "x", "c", "y"];
    let ratio = SequenceMatcher::new(&a, &b).ratio();
    assert!((ratio - 0.5).abs() < 1e-9, "expected 0.5, got {}", ratio);
}

#[test]
fn test_ratio_longer_strings() {
    // Python: difflib.SequenceMatcher(None, "hello world", "hallo werld").ratio()
    //   matches: h, l, l, o, ' ', w, r, l, d = 9 chars, len=11+11=22, 2*9/22 ≈ 0.8182
    let a: Vec<char> = "hello world".chars().collect();
    let b: Vec<char> = "hallo werld".chars().collect();
    let ratio = SequenceMatcher::new(&a, &b).ratio();
    // Python 实际值约为 0.8182 (verified: 0.8181818181818182)
    assert!((ratio - 0.8182).abs() < 1e-3, "expected ~0.8182, got {}", ratio);
}
