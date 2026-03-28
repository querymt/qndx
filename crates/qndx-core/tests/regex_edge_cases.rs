//! Correctness test suite for regex edge cases (Issue #6).
//!
//! Covers: alternation, character classes, escapes, anchors, repetition,
//! empty matches, overlapping matches, and Unicode.
//!
//! This suite serves as the correctness oracle for all search paths.
//! Index-backed search (M2+) must produce identical results.

use qndx_core::scan::{scan_content, scan_search, SearchMatch};
use qndx_core::walk::WalkConfig;
use std::fs;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn matches_for(pattern: &str, content: &str) -> Vec<SearchMatch> {
    scan_content(pattern, "test.rs", content).unwrap()
}

fn matched_texts(pattern: &str, content: &str) -> Vec<String> {
    matches_for(pattern, content)
        .into_iter()
        .map(|m| m.text)
        .collect()
}

// ---------------------------------------------------------------------------
// Alternation
// ---------------------------------------------------------------------------

#[test]
fn alternation_simple() {
    let texts = matched_texts("a|b", "a b c a");
    assert_eq!(texts, vec!["a", "b", "a"]);
}

#[test]
fn alternation_words() {
    let texts = matched_texts("foo|bar", "foo baz bar foo");
    assert_eq!(texts, vec!["foo", "bar", "foo"]);
}

#[test]
fn alternation_grouped() {
    let texts = matched_texts("(foo|bar)baz", "foobaz barbaz qux");
    assert_eq!(texts, vec!["foobaz", "barbaz"]);
}

#[test]
fn alternation_no_match() {
    let texts = matched_texts("cat|dog", "bird fish");
    assert!(texts.is_empty());
}

#[test]
fn alternation_overlapping_prefixes() {
    let texts = matched_texts("abc|abcd", "abcd");
    // Regex engine matches leftmost first: "abc" at position 0
    assert_eq!(texts, vec!["abc"]);
}

// ---------------------------------------------------------------------------
// Character classes
// ---------------------------------------------------------------------------

#[test]
fn char_class_range() {
    let texts = matched_texts("[a-z]+", "Hello World 123");
    assert_eq!(texts, vec!["ello", "orld"]);
}

#[test]
fn char_class_negated() {
    let texts = matched_texts("[^0-9]+", "abc123def");
    assert_eq!(texts, vec!["abc", "def"]);
}

#[test]
fn char_class_digits() {
    let texts = matched_texts("[0-9]+", "foo42bar99");
    assert_eq!(texts, vec!["42", "99"]);
}

#[test]
fn char_class_custom_set() {
    let texts = matched_texts("[aeiou]+", "beautiful");
    assert_eq!(texts, vec!["eau", "i", "u"]);
}

#[test]
fn char_class_unicode_letter() {
    // \p{L} matches Unicode letters
    let texts = matched_texts(r"\p{L}+", "hello 世界 cafe");
    assert_eq!(texts, vec!["hello", "世界", "cafe"]);
}

// ---------------------------------------------------------------------------
// Escapes
// ---------------------------------------------------------------------------

#[test]
fn escape_literal_dot() {
    let texts = matched_texts(r"\.", "a.b.c");
    assert_eq!(texts, vec![".", "."]);
}

#[test]
fn escape_digit() {
    let texts = matched_texts(r"\d+", "abc 42 def");
    assert_eq!(texts, vec!["42"]);
}

#[test]
fn escape_word_char() {
    let texts = matched_texts(r"\w+", "hello-world_42");
    assert_eq!(texts, vec!["hello", "world_42"]);
}

#[test]
fn escape_whitespace() {
    let texts = matched_texts(r"\s+", "a  b\tc");
    assert_eq!(texts, vec!["  ", "\t"]);
}

#[test]
fn escape_word_boundary() {
    let texts = matched_texts(r"\bfn\b", "fn foo fn_bar fn");
    // \bfn\b should match "fn" as whole word, not "fn" in "fn_bar"
    assert_eq!(texts, vec!["fn", "fn"]);
}

#[test]
fn escape_backslash_literal() {
    let texts = matched_texts(r"\\", r"a\b\c");
    assert_eq!(texts, vec![r"\", r"\"]);
}

// ---------------------------------------------------------------------------
// Anchors
// ---------------------------------------------------------------------------

#[test]
fn anchor_start_of_line() {
    let texts = matched_texts("(?m)^fn", "fn main\n  fn helper\nfn other");
    assert_eq!(texts, vec!["fn", "fn"]);
}

#[test]
fn anchor_end_of_line() {
    let texts = matched_texts(r"(?m);$", "let x = 1;\nlet y = 2;\nno semi");
    assert_eq!(texts, vec![";", ";"]);
}

#[test]
fn anchor_start_of_string() {
    let texts = matched_texts("^hello", "hello world\nhello again");
    // Without (?m), ^ only matches start of entire string
    assert_eq!(texts, vec!["hello"]);
}

#[test]
fn anchor_end_of_string() {
    let texts = matched_texts("end$", "start\nthe end");
    // Without (?m), $ only matches end of entire string
    assert_eq!(texts, vec!["end"]);
}

#[test]
fn anchor_multiline_combined() {
    let content = "fn one()\nfn two()\nlet three()";
    let texts = matched_texts(r"(?m)^fn \w+\(\)$", content);
    assert_eq!(texts, vec!["fn one()", "fn two()"]);
}

// ---------------------------------------------------------------------------
// Repetition
// ---------------------------------------------------------------------------

#[test]
fn repetition_star() {
    let texts = matched_texts("ab*c", "ac abc abbc xabbbc");
    assert_eq!(texts, vec!["ac", "abc", "abbc", "abbbc"]);
}

#[test]
fn repetition_plus() {
    let texts = matched_texts("ab+c", "ac abc abbc");
    assert_eq!(texts, vec!["abc", "abbc"]);
}

#[test]
fn repetition_question() {
    let texts = matched_texts("colou?r", "color colour");
    assert_eq!(texts, vec!["color", "colour"]);
}

#[test]
fn repetition_exact_count() {
    let texts = matched_texts(r"\d{3}", "1 12 123 1234");
    assert_eq!(texts, vec!["123", "123"]);
}

#[test]
fn repetition_range() {
    let texts = matched_texts(r"\d{2,4}", "1 12 123 1234 12345");
    // Greedy: matches longest
    assert_eq!(texts, vec!["12", "123", "1234", "1234"]);
}

#[test]
fn repetition_lazy() {
    let texts = matched_texts(r"\d{2,4}?", "12345");
    // Lazy: matches shortest (2 digits)
    assert_eq!(texts, vec!["12", "34"]);
}

// ---------------------------------------------------------------------------
// Empty matches
// ---------------------------------------------------------------------------

#[test]
fn empty_match_pattern() {
    // Pattern that can match empty string; regex crate skips empty matches
    // at positions where a non-empty match also occurs.
    let matches = matches_for("a*", "ba");
    // Rust regex: at pos 0 matches "" (no 'a'), at pos 1 matches "a",
    // at pos 2 matches "" (end). Non-empty matches win at their position.
    assert!(matches.iter().any(|m| m.text == "a"));
}

#[test]
fn empty_string_no_match() {
    let matches = matches_for("abc", "");
    assert!(matches.is_empty());
}

// ---------------------------------------------------------------------------
// Overlapping / adjacent matches
// ---------------------------------------------------------------------------

#[test]
fn adjacent_matches() {
    let texts = matched_texts(r"\d+", "123abc456");
    assert_eq!(texts, vec!["123", "456"]);
}

#[test]
fn non_overlapping_greedy() {
    // Greedy match consumes as much as possible
    let texts = matched_texts("a+", "aaa a aa");
    assert_eq!(texts, vec!["aaa", "a", "aa"]);
}

// ---------------------------------------------------------------------------
// Complex / realistic patterns
// ---------------------------------------------------------------------------

#[test]
fn pattern_function_signature() {
    let content = "fn main() {\n    fn helper(x: u32) -> bool {\n        true\n    }\n}\n";
    let texts = matched_texts(r"fn \w+\([^)]*\)", content);
    assert_eq!(texts, vec!["fn main()", "fn helper(x: u32)"]);
}

#[test]
fn pattern_rust_use_statement() {
    let content = "use std::collections::HashMap;\nuse regex::Regex;\nlet x = 1;\n";
    let texts = matched_texts(r"use [\w:]+;", content);
    assert_eq!(
        texts,
        vec!["use std::collections::HashMap;", "use regex::Regex;"]
    );
}

#[test]
fn pattern_identifier_with_underscore() {
    let content = "let MAX_FILE_SIZE = 1024;\nlet min_size = 0;\nlet maxSize = 10;\n";
    let texts = matched_texts(r"[A-Z][A-Z_]+", content);
    assert_eq!(texts, vec!["MAX_FILE_SIZE"]);
}

#[test]
fn pattern_ip_address_like() {
    let content = "addr: 192.168.1.1\nport: 8080\nother: 10.0.0.1\n";
    let texts = matched_texts(r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}", content);
    assert_eq!(texts, vec!["192.168.1.1", "10.0.0.1"]);
}

// ---------------------------------------------------------------------------
// Unicode
// ---------------------------------------------------------------------------

#[test]
fn unicode_basic_match() {
    let texts = matched_texts("世界", "hello 世界 world");
    assert_eq!(texts, vec!["世界"]);
}

#[test]
fn unicode_in_class() {
    let texts = matched_texts("[a-z世界]+", "hello世界abc");
    assert_eq!(texts, vec!["hello世界abc"]);
}

#[test]
fn unicode_column_positions() {
    let content = "ab世界cd";
    let matches = matches_for("cd", content);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].line, 1);
    // Column is byte-offset based: 'a'=1byte, 'b'=1byte, '世'=3bytes, '界'=3bytes
    // so "cd" starts at byte offset 8, column = 9
    assert_eq!(matches[0].column, 9);
}

// ---------------------------------------------------------------------------
// Full scan search with file system
// ---------------------------------------------------------------------------

#[test]
fn full_scan_search_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    fs::write(root.join("a.rs"), "fn foo() {}\nfn bar() {}\n").unwrap();
    fs::write(root.join("b.rs"), "fn baz() {}\n").unwrap();

    let r1 = scan_search(root, r"fn \w+\(\)", &WalkConfig::default()).unwrap();
    let r2 = scan_search(root, r"fn \w+\(\)", &WalkConfig::default()).unwrap();

    assert_eq!(r1.matches.len(), 3);
    assert_eq!(r1.matches, r2.matches);
    assert_eq!(r1.files_scanned, 2);
}

#[test]
fn full_scan_no_matches() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.rs"), "nothing here").unwrap();

    let results = scan_search(dir.path(), "NONEXISTENT_PATTERN", &WalkConfig::default()).unwrap();
    assert!(results.matches.is_empty());
    assert_eq!(results.files_scanned, 1);
}

// ---------------------------------------------------------------------------
// Baseline latency recording (not asserted, just exercised for future use)
// ---------------------------------------------------------------------------

#[test]
fn baseline_latency_exercise() {
    // This test exercises the search path so that when run under criterion
    // or manual timing, we have a baseline. We just verify correctness here.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // Create a modest corpus
    for i in 0..20 {
        let content = format!(
            "fn func_{i}() {{\n    let x_{i} = MAX_FILE_SIZE;\n    process_batch({i});\n}}\n"
        );
        fs::write(root.join(format!("file_{i:03}.rs")), content).unwrap();
    }

    let patterns = vec![
        ("literal", "MAX_FILE_SIZE"),
        ("alternation", "process_batch|func_0"),
        ("class", "func_[0-9]+"),
        ("wildcard", "let x_.*="),
        ("digit", r"process_batch\(\d+\)"),
        ("word_boundary", r"\blet\b"),
        ("complex", r"fn func_\d+\(\)"),
    ];

    for (name, pattern) in &patterns {
        let results = scan_search(root, pattern, &WalkConfig::default()).unwrap();
        assert!(
            !results.matches.is_empty(),
            "pattern '{}' ({}) should match",
            pattern,
            name
        );
    }
}
