//! Differential tests: index-backed vs scan-only results (Issue #25).
//!
//! This harness verifies that index-backed search results are a superset of
//! (or equal to) scan-only search results for any given query and corpus.
//!
//! Now uses the real M2 index-backed search pipeline:
//! build index -> decompose pattern -> lookup postings -> verify candidates.

use qndx_core::scan::{scan_matching_files, scan_search};
use qndx_core::walk::WalkConfig;
use qndx_index::{build_index, IndexReader};
use qndx_query::index_search_matching_files;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

/// Result of a differential comparison.
#[derive(Debug)]
struct DiffResult {
    pattern: String,
    /// Files found by scan-only (ground truth).
    scan_files: BTreeSet<String>,
    /// Files found by index-backed search.
    index_files: BTreeSet<String>,
    /// Files in scan but missing from index (false negatives -- must be empty).
    false_negatives: BTreeSet<String>,
    /// Files in index but not in scan (false positives -- acceptable).
    #[allow(dead_code)]
    false_positives: BTreeSet<String>,
}

/// Build an index from the corpus and run index-backed file matching.
/// Uses a separate temp directory for the index to avoid contaminating the corpus.
fn build_and_search_index(root: &Path, pattern: &str, config: &WalkConfig) -> BTreeSet<String> {
    let index_tmp = tempfile::tempdir().unwrap();
    let index_dir = index_tmp.path().join("v1");
    let files = qndx_core::walk::discover_and_read_files(root, config);
    build_index(&files, &index_dir, None).unwrap();

    let reader = IndexReader::open(&index_dir).unwrap();
    index_search_matching_files(&reader, root, pattern)
        .unwrap()
        .into_iter()
        .collect()
}

/// Run both search paths and compare.
fn differential_compare(root: &Path, pattern: &str, config: &WalkConfig) -> DiffResult {
    let scan_files: BTreeSet<String> = scan_matching_files(root, pattern, config)
        .unwrap()
        .into_iter()
        .collect();

    let index_files: BTreeSet<String> = build_and_search_index(root, pattern, config);

    let false_negatives: BTreeSet<String> = scan_files.difference(&index_files).cloned().collect();
    let false_positives: BTreeSet<String> = index_files.difference(&scan_files).cloned().collect();

    DiffResult {
        pattern: pattern.to_string(),
        scan_files,
        index_files,
        false_negatives,
        false_positives,
    }
}

/// Assert zero false negatives.
fn assert_no_false_negatives(result: &DiffResult) {
    assert!(
        result.false_negatives.is_empty(),
        "FALSE NEGATIVES for pattern '{}': {:?}\n  scan found: {:?}\n  index found: {:?}",
        result.pattern,
        result.false_negatives,
        result.scan_files,
        result.index_files,
    );
}

// ---------------------------------------------------------------------------
// Corpus builders
// ---------------------------------------------------------------------------

fn build_small_corpus(root: &Path) {
    fs::write(
        root.join("main.rs"),
        "fn main() {\n    let x = MAX_FILE_SIZE;\n    println!(\"hello world\");\n}\n",
    )
    .unwrap();

    fs::write(
        root.join("lib.rs"),
        "pub const MAX_FILE_SIZE: usize = 1024;\npub fn parse_config() -> bool { true }\n",
    )
    .unwrap();

    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/util.rs"),
        "use std::collections::HashMap;\nfn helper(x: u32) -> u32 { x + 1 }\npub fn serialize_data() {}\n",
    )
    .unwrap();

    fs::write(
        root.join("src/handler.rs"),
        "pub fn handle_request() {}\npub fn handle_response() {}\n",
    )
    .unwrap();
}

fn build_medium_corpus(root: &Path) {
    build_small_corpus(root);

    for i in 0..50 {
        let content = format!(
            "mod module_{i:03};\nfn func_{i}(x: u32) -> u32 {{\n    let result = x * {i};\n    result\n}}\n\
             const VALUE_{i}: usize = {val};\npub struct Type_{i} {{ field: String }}\n",
            i = i,
            val = i * 100,
        );
        fs::create_dir_all(root.join(format!("src/mod_{:02}", i / 10))).unwrap();
        fs::write(
            root.join(format!("src/mod_{:02}/file_{:03}.rs", i / 10, i)),
            content,
        )
        .unwrap();
    }
}

fn build_large_corpus(root: &Path) {
    build_medium_corpus(root);

    for i in 50..200 {
        let content = format!(
            "//! Module {i}\nuse crate::Type_0;\n\
             fn process_{i}(input: &str) -> Result<(), String> {{\n    \
                 let MAX_FILE_SIZE = 4096;\n    \
                 if input.len() > MAX_FILE_SIZE {{ return Err(\"too large\".into()) }}\n    \
                 Ok(())\n}}\n\
             #[test]\nfn test_{i}() {{ assert!(process_{i}(\"hello\").is_ok()) }}\n",
            i = i,
        );
        fs::create_dir_all(root.join(format!("src/mod_{:02}", i / 10))).unwrap();
        fs::write(
            root.join(format!("src/mod_{:02}/file_{:03}.rs", i / 10, i)),
            content,
        )
        .unwrap();
    }
}

// ---------------------------------------------------------------------------
// Query suites
// ---------------------------------------------------------------------------

fn representative_queries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("literal_simple", "MAX_FILE_SIZE"),
        ("literal_fn", "fn main"),
        ("alternation", "parse_config|serialize_data"),
        ("char_class", "func_[0-9]+"),
        ("wildcard", "handle_.*"),
        ("digit", r"VALUE_\d+"),
        ("word_boundary", r"\bfn\b"),
        ("complex", r"fn \w+\([^)]*\)"),
        ("anchored", "(?m)^pub "),
        ("no_match", "ZZZNONEXISTENT999"),
    ]
}

// ---------------------------------------------------------------------------
// Differential tests
// ---------------------------------------------------------------------------

#[test]
fn differential_small_corpus() {
    let dir = tempfile::tempdir().unwrap();
    build_small_corpus(dir.path());
    let config = WalkConfig::default();

    for (_name, pattern) in representative_queries() {
        let result = differential_compare(dir.path(), pattern, &config);
        assert_no_false_negatives(&result);
    }
}

#[test]
fn differential_medium_corpus() {
    let dir = tempfile::tempdir().unwrap();
    build_medium_corpus(dir.path());
    let config = WalkConfig::default();

    for (_name, pattern) in representative_queries() {
        let result = differential_compare(dir.path(), pattern, &config);
        assert_no_false_negatives(&result);
    }
}

#[test]
fn differential_large_corpus() {
    let dir = tempfile::tempdir().unwrap();
    build_large_corpus(dir.path());
    let config = WalkConfig::default();

    for (_name, pattern) in representative_queries() {
        let result = differential_compare(dir.path(), pattern, &config);
        assert_no_false_negatives(&result);
    }
}

// ---------------------------------------------------------------------------
// Corpus integrity checksums
// ---------------------------------------------------------------------------

/// Compute a simple checksum of all file contents for reproducibility.
fn corpus_checksum(root: &Path, config: &WalkConfig) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let files = qndx_core::walk::discover_files(root, config);
    let mut hasher = DefaultHasher::new();

    for file in &files {
        file.rel_path.hash(&mut hasher);
        if let Ok(content) = fs::read(&file.abs_path) {
            content.hash(&mut hasher);
        }
    }

    hasher.finish()
}

#[test]
fn corpus_checksum_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    build_medium_corpus(dir.path());
    let config = WalkConfig::default();

    let c1 = corpus_checksum(dir.path(), &config);
    let c2 = corpus_checksum(dir.path(), &config);
    assert_eq!(c1, c2, "corpus checksum must be deterministic");
}

// ---------------------------------------------------------------------------
// Match-level differential (positions, not just files)
// ---------------------------------------------------------------------------

#[test]
fn differential_match_positions_small() {
    let dir = tempfile::tempdir().unwrap();
    build_small_corpus(dir.path());
    let config = WalkConfig::default();

    // Build index in a separate temp directory to avoid contaminating the corpus
    let index_tmp = tempfile::tempdir().unwrap();
    let index_dir = index_tmp.path().join("v1");
    let files = qndx_core::walk::discover_and_read_files(dir.path(), &config);
    build_index(&files, &index_dir, None).unwrap();

    for (_name, pattern) in representative_queries() {
        let scan_results = scan_search(dir.path(), pattern, &config).unwrap();

        // Compare against index-backed match extraction
        let index_results =
            qndx_query::index_search(dir.path(), &index_dir, pattern).unwrap();

        // Every scan match must appear in index results (no false negatives)
        for scan_match in &scan_results.matches {
            assert!(
                index_results.results.matches.contains(scan_match),
                "FALSE NEGATIVE (match-level) for pattern '{}': {:?} missing from index results",
                pattern,
                scan_match,
            );
        }
    }
}
