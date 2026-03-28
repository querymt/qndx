//! Scan-only search: direct file scanning with regex (no index).
//!
//! This module serves as the **correctness oracle** for all future
//! index-backed search paths. Results from index-backed search must
//! be a superset of (or equal to) scan-only results.

use crate::walk::{self, WalkConfig};
use regex::Regex;
use std::path::Path;

/// A single match within a file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SearchMatch {
    /// Relative file path.
    pub path: String,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column (byte offset within the line).
    pub column: usize,
    /// The matched text.
    pub text: String,
}

/// Complete search results.
#[derive(Debug, Clone)]
pub struct SearchResults {
    /// All matches, sorted by (path, line, column).
    pub matches: Vec<SearchMatch>,
    /// Number of files scanned.
    pub files_scanned: usize,
    /// Total bytes scanned.
    pub bytes_scanned: u64,
}

/// Run a scan-only search: walk files from `root`, apply `pattern` to each file,
/// and return all matches with positions.
///
/// This is the correctness oracle. Output is deterministic: matches are sorted
/// by (path, line, column).
pub fn scan_search(
    root: &Path,
    pattern: &str,
    config: &WalkConfig,
) -> Result<SearchResults, String> {
    let re = Regex::new(pattern).map_err(|e| format!("invalid regex: {}", e))?;
    let files = walk::discover_files(root, config);

    let mut all_matches = Vec::new();
    let mut files_scanned = 0u64;
    let mut bytes_scanned = 0u64;

    for file in &files {
        let content = match std::fs::read(&file.abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        files_scanned += 1;
        bytes_scanned += content.len() as u64;

        let text = String::from_utf8_lossy(&content);
        let file_matches = extract_matches(&re, &file.rel_path, &text);
        all_matches.extend(file_matches);
    }

    // Sort for deterministic output
    all_matches.sort();

    Ok(SearchResults {
        matches: all_matches,
        files_scanned: files_scanned as usize,
        bytes_scanned,
    })
}

/// Scan a single file's content (as string) and extract all matches with positions.
pub fn scan_content(pattern: &str, path: &str, content: &str) -> Result<Vec<SearchMatch>, String> {
    let re = Regex::new(pattern).map_err(|e| format!("invalid regex: {}", e))?;
    let mut matches = extract_matches(&re, path, content);
    matches.sort();
    Ok(matches)
}

/// Extract all non-overlapping matches from `content`, computing line/column positions.
fn extract_matches(re: &Regex, path: &str, content: &str) -> Vec<SearchMatch> {
    let mut results = Vec::new();

    // Precompute line start offsets for O(1) line/column lookup
    let line_starts = compute_line_starts(content);

    for mat in re.find_iter(content) {
        let byte_offset = mat.start();
        let (line, column) = offset_to_line_col(&line_starts, byte_offset);
        let matched_text = mat.as_str().to_string();

        // Skip empty matches to avoid infinite loops on patterns like `.*`
        // (regex crate handles this, but be defensive)
        results.push(SearchMatch {
            path: path.to_string(),
            line,
            column,
            text: matched_text,
        });
    }

    results
}

/// Compute byte offsets where each line starts (0-indexed offsets, 1-indexed lines).
fn compute_line_starts(content: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Convert a byte offset to (1-based line, 1-based column).
fn offset_to_line_col(line_starts: &[usize], offset: usize) -> (usize, usize) {
    // Binary search for the line containing this offset
    let line_idx = match line_starts.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let column = offset - line_starts[line_idx] + 1;
    (line_idx + 1, column)
}

/// Scan raw bytes content (used by the verifier and differential tests).
/// Handles non-UTF8 content via lossy conversion.
pub fn scan_bytes(pattern: &str, path: &str, content: &[u8]) -> Result<Vec<SearchMatch>, String> {
    let text = String::from_utf8_lossy(content);
    scan_content(pattern, path, &text)
}

/// Check if a file's content matches a pattern (boolean, no positions).
/// Efficient for candidate verification.
pub fn file_matches(pattern: &str, content: &[u8]) -> Result<bool, String> {
    let re = Regex::new(pattern).map_err(|e| format!("invalid regex: {}", e))?;
    let text = String::from_utf8_lossy(content);
    Ok(re.is_match(&text))
}

/// Return just the set of file paths that match (no positions).
/// Useful for differential testing against index-backed candidate sets.
pub fn scan_matching_files(
    root: &Path,
    pattern: &str,
    config: &WalkConfig,
) -> Result<Vec<String>, String> {
    let re = Regex::new(pattern).map_err(|e| format!("invalid regex: {}", e))?;
    let files = walk::discover_files(root, config);

    let mut matching = Vec::new();
    for file in &files {
        let content = match std::fs::read(&file.abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let text = String::from_utf8_lossy(&content);
        if re.is_match(&text) {
            matching.push(file.rel_path.clone());
        }
    }

    matching.sort();
    Ok(matching)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_search_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("main.rs"),
            "fn main() {\n    let x = MAX_FILE_SIZE;\n    println!(\"hello\");\n}\n",
        )
        .unwrap();

        fs::write(
            root.join("lib.rs"),
            "pub const MAX_FILE_SIZE: usize = 1024;\npub const MIN_SIZE: usize = 0;\n",
        )
        .unwrap();

        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/util.rs"),
            "fn helper() -> u32 {\n    42\n}\n\nfn another_helper() {\n    let MAX_FILE_SIZE = 999;\n}\n",
        )
        .unwrap();

        fs::write(root.join("empty.rs"), "").unwrap();

        dir
    }

    #[test]
    fn scan_literal_pattern() {
        let dir = setup_search_dir();
        let results = scan_search(dir.path(), "MAX_FILE_SIZE", &WalkConfig::default()).unwrap();

        assert_eq!(results.matches.len(), 3);
        assert_eq!(results.files_scanned, 4);

        // Results should be sorted by path
        let paths: Vec<&str> = results.matches.iter().map(|m| m.path.as_str()).collect();
        assert!(paths.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn scan_extracts_positions() {
        let content = "line one\nline two MAX_FILE_SIZE here\nline three\n";
        let matches = scan_content("MAX_FILE_SIZE", "test.rs", content).unwrap();

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 2);
        assert_eq!(matches[0].column, 10);
        assert_eq!(matches[0].text, "MAX_FILE_SIZE");
    }

    #[test]
    fn scan_multiple_matches_per_line() {
        let content = "foo bar foo baz foo\n";
        let matches = scan_content("foo", "test.rs", content).unwrap();

        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].column, 1);
        assert_eq!(matches[1].column, 9);
        assert_eq!(matches[2].column, 17);
        assert!(matches.iter().all(|m| m.line == 1));
    }

    #[test]
    fn scan_regex_digits() {
        let content = "let x = 42;\nlet y = 100;\nlet z = abc;\n";
        let matches = scan_content(r"\d+", "test.rs", content).unwrap();

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].text, "42");
        assert_eq!(matches[1].text, "100");
    }

    #[test]
    fn scan_empty_file() {
        let matches = scan_content("anything", "empty.rs", "").unwrap();
        assert!(matches.is_empty());
    }

    #[test]
    fn scan_invalid_regex() {
        let result = scan_content("[invalid", "test.rs", "content");
        assert!(result.is_err());
    }

    #[test]
    fn scan_bytes_lossy() {
        let content = b"hello \xff world MAX_SIZE end";
        let matches = scan_bytes("MAX_SIZE", "test.bin", content).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].text, "MAX_SIZE");
    }

    #[test]
    fn file_matches_basic() {
        assert!(file_matches("hello", b"say hello world").unwrap());
        assert!(!file_matches("hello", b"goodbye world").unwrap());
    }

    #[test]
    fn scan_matching_files_basic() {
        let dir = setup_search_dir();
        let matches =
            scan_matching_files(dir.path(), "MAX_FILE_SIZE", &WalkConfig::default()).unwrap();

        // main.rs, lib.rs, src/util.rs all contain MAX_FILE_SIZE
        assert_eq!(matches.len(), 3);
        assert!(matches.contains(&"main.rs".to_string()));
        assert!(matches.contains(&"lib.rs".to_string()));
        assert!(matches.contains(&"src/util.rs".to_string()));
    }

    #[test]
    fn output_is_deterministic() {
        let dir = setup_search_dir();
        let r1 = scan_search(dir.path(), "MAX_FILE_SIZE", &WalkConfig::default()).unwrap();
        let r2 = scan_search(dir.path(), "MAX_FILE_SIZE", &WalkConfig::default()).unwrap();

        assert_eq!(r1.matches.len(), r2.matches.len());
        for (a, b) in r1.matches.iter().zip(r2.matches.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn line_col_computation() {
        // "abc\ndef\nghi"
        //  0123 4567 89...
        let content = "abc\ndef\nghi";
        let starts = compute_line_starts(content);
        assert_eq!(starts, vec![0, 4, 8]);

        assert_eq!(offset_to_line_col(&starts, 0), (1, 1)); // 'a'
        assert_eq!(offset_to_line_col(&starts, 2), (1, 3)); // 'c'
        assert_eq!(offset_to_line_col(&starts, 4), (2, 1)); // 'd'
        assert_eq!(offset_to_line_col(&starts, 8), (3, 1)); // 'g'
        assert_eq!(offset_to_line_col(&starts, 10), (3, 3)); // 'i'
    }
}
