//! Index-backed search pipeline: decompose -> lookup -> verify -> results.
//!
//! This is the core search path for M2. It:
//! 1. Decomposes a regex pattern into required trigram hashes
//! 2. Queries the index to produce a candidate file set
//! 3. Verifies candidates by running the original regex against file content
//! 4. Returns only verified matches (no false negatives)

use std::path::Path;

use qndx_core::scan::{SearchMatch, SearchResults};
use qndx_index::IndexReader;

use crate::planner::{plan_query, QueryPlan};

/// Statistics from an index-backed search.
#[derive(Debug, Clone)]
pub struct IndexSearchStats {
    /// Total files in the index.
    pub total_files: u32,
    /// Number of candidate files after index filtering.
    pub candidate_count: usize,
    /// Number of files that actually matched after verification.
    pub verified_count: usize,
    /// Number of n-gram lookups performed.
    pub lookup_count: usize,
    /// Which strategy the planner selected.
    pub strategy: crate::planner::PlanStrategy,
}

/// Result of an index-backed search including stats.
#[derive(Debug, Clone)]
pub struct IndexSearchResults {
    /// The verified search results.
    pub results: SearchResults,
    /// Statistics about the search pipeline.
    pub stats: IndexSearchStats,
}

/// Run an index-backed search.
///
/// 1. Decompose the pattern into trigram hashes
/// 2. Query the index for candidate files
/// 3. Read and verify each candidate against the actual regex
/// 4. Return verified matches
///
/// `root` is the directory where the original files live (for reading content).
/// `index_dir` is where ngrams.tbl / postings.dat / manifest.bin are stored.
pub fn index_search(
    root: &Path,
    index_dir: &Path,
    pattern: &str,
) -> Result<IndexSearchResults, String> {
    let reader =
        IndexReader::open(index_dir).map_err(|e| format!("failed to open index: {}", e))?;

    index_search_with_reader(&reader, root, pattern)
}

/// Run an index-backed search using a pre-loaded IndexReader.
pub fn index_search_with_reader(
    reader: &IndexReader,
    root: &Path,
    pattern: &str,
) -> Result<IndexSearchResults, String> {
    let re = regex::Regex::new(pattern).map_err(|e| format!("invalid regex: {}", e))?;

    // Step 1: Plan the query (chooses trigram vs sparse strategy)
    let plan = plan_query(pattern);

    // Step 2: Get candidate set from index using the plan's chosen hashes
    let candidates = resolve_candidates_from_plan(reader, &plan);

    let candidate_ids = candidates.to_vec();
    let candidate_count = candidate_ids.len();

    // Step 3: Verify candidates by reading actual file content
    let mut all_matches = Vec::new();
    let mut files_scanned = 0usize;
    let mut bytes_scanned = 0u64;
    let mut verified_count = 0usize;

    for &file_id in &candidate_ids {
        let rel_path = match reader.file_path(file_id) {
            Some(p) => p,
            None => continue,
        };

        let abs_path = root.join(rel_path);
        let content = match std::fs::read(&abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        files_scanned += 1;
        bytes_scanned += content.len() as u64;

        let text = String::from_utf8_lossy(&content);
        if re.is_match(&text) {
            verified_count += 1;
            // Extract match positions
            let file_matches = extract_matches(&re, rel_path, &text);
            all_matches.extend(file_matches);
        }
    }

    all_matches.sort();

    Ok(IndexSearchResults {
        results: SearchResults {
            matches: all_matches,
            files_scanned,
            bytes_scanned,
        },
        stats: IndexSearchStats {
            total_files: reader.file_count(),
            candidate_count,
            verified_count,
            lookup_count: plan.lookup_count,
            strategy: plan.strategy,
        },
    })
}

/// Return just the set of matching file paths (no positions) via index-backed search.
/// Used for differential testing.
pub fn index_search_matching_files(
    reader: &IndexReader,
    root: &Path,
    pattern: &str,
) -> Result<Vec<String>, String> {
    let re = regex::Regex::new(pattern).map_err(|e| format!("invalid regex: {}", e))?;

    let plan = plan_query(pattern);
    let candidates = resolve_candidates_from_plan(reader, &plan);

    let candidate_ids = candidates.to_vec();
    let mut matching = Vec::new();

    for &file_id in &candidate_ids {
        let rel_path = match reader.file_path(file_id) {
            Some(p) => p,
            None => continue,
        };

        let abs_path = root.join(rel_path);
        let content = match std::fs::read(&abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let text = String::from_utf8_lossy(&content);
        if re.is_match(&text) {
            matching.push(rel_path.to_string());
        }
    }

    matching.sort();
    Ok(matching)
}

/// Plan a query and return the plan (for diagnostics / benchmarking).
pub fn plan(pattern: &str) -> QueryPlan {
    plan_query(pattern)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

use qndx_index::postings::PostingList;

/// Resolve candidate file set from a query plan.
///
/// Uses the plan's `required_hashes` and `alternative_hashes` (which may come
/// from either the trigram or sparse strategy, depending on what the planner chose).
///
/// - If no hashes at all: all files are candidates (fallback to full scan).
/// - If only required hashes (no alternation): intersect them (AND).
/// - If only alternatives (top-level alternation): union each branch's intersection.
/// - If both required and alternatives: intersect required, then intersect with
///   the union of alternatives.
fn resolve_candidates_from_plan(reader: &IndexReader, plan: &QueryPlan) -> PostingList {
    let has_required = !plan.required_hashes.is_empty();
    let has_alternatives = !plan.alternative_hashes.is_empty();

    if !has_required && !has_alternatives {
        // No n-grams extracted: all files are candidates
        let all: Vec<qndx_core::FileId> = (0..reader.file_count()).collect();
        return PostingList::from_vec(all);
    }

    if has_required && !has_alternatives {
        // Simple case: intersect all required n-grams
        return reader.lookup_intersect(&plan.required_hashes);
    }

    // Build union of all alternative branches
    let mut alt_union = PostingList::from_vec(vec![]);
    for alt_hashes in &plan.alternative_hashes {
        if alt_hashes.is_empty() {
            // Branch with no extractable n-grams: all files are candidates for this branch
            let all: Vec<qndx_core::FileId> = (0..reader.file_count()).collect();
            alt_union = PostingList::from_vec(all);
            break; // Union with "all" is "all"
        }
        let branch_result = reader.lookup_intersect(alt_hashes);
        alt_union = alt_union.union(&branch_result);
    }

    if has_required {
        // Intersect required n-grams with the union of alternatives
        let required_result = reader.lookup_intersect(&plan.required_hashes);
        required_result.intersect(&alt_union)
    } else {
        // Only alternatives (top-level alternation, no shared required n-grams)
        alt_union
    }
}

/// Extract all non-overlapping matches with line/column positions.
fn extract_matches(re: &regex::Regex, path: &str, content: &str) -> Vec<SearchMatch> {
    let mut results = Vec::new();
    let line_starts = compute_line_starts(content);

    for mat in re.find_iter(content) {
        let byte_offset = mat.start();
        let (line, column) = offset_to_line_col(&line_starts, byte_offset);

        results.push(SearchMatch {
            path: path.to_string(),
            line,
            column,
            text: mat.as_str().to_string(),
        });
    }

    results
}

fn compute_line_starts(content: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

fn offset_to_line_col(line_starts: &[usize], offset: usize) -> (usize, usize) {
    let line_idx = match line_starts.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let column = offset - line_starts[line_idx] + 1;
    (line_idx + 1, column)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qndx_core::walk::WalkConfig;
    use qndx_index::build_index;
    use std::fs;

    fn setup_corpus_and_index() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("main.rs"),
            "fn main() {\n    let x = MAX_FILE_SIZE;\n    println!(\"hello\");\n}\n",
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
            "fn helper() -> u32 { 42 }\npub fn serialize_data() {}\n",
        )
        .unwrap();

        // Build index
        let index_dir = root.join("index/v1");
        let files = qndx_core::walk::discover_and_read_files(root, &WalkConfig::default());
        build_index(&files, &index_dir, None).unwrap();

        (dir, index_dir)
    }

    #[test]
    fn index_search_literal() {
        let (dir, index_dir) = setup_corpus_and_index();
        let result = index_search(dir.path(), &index_dir, "MAX_FILE_SIZE").unwrap();

        assert!(!result.results.matches.is_empty());
        // Should find in main.rs and lib.rs
        let paths: Vec<&str> = result
            .results
            .matches
            .iter()
            .map(|m| m.path.as_str())
            .collect();
        assert!(paths.contains(&"main.rs"));
        assert!(paths.contains(&"lib.rs"));
        assert!(!paths.contains(&"src/util.rs"));
    }

    #[test]
    fn index_search_regex() {
        let (dir, index_dir) = setup_corpus_and_index();
        let result = index_search(dir.path(), &index_dir, r"fn \w+\(\)").unwrap();

        assert!(!result.results.matches.is_empty());
    }

    #[test]
    fn index_search_no_match() {
        let (dir, index_dir) = setup_corpus_and_index();
        let result = index_search(dir.path(), &index_dir, "NONEXISTENT_PATTERN_XYZ").unwrap();

        assert!(result.results.matches.is_empty());
    }

    #[test]
    fn index_search_matching_files_basic() {
        let (dir, index_dir) = setup_corpus_and_index();
        let reader = IndexReader::open(&index_dir).unwrap();
        let matching = index_search_matching_files(&reader, dir.path(), "MAX_FILE_SIZE").unwrap();

        assert!(matching.contains(&"main.rs".to_string()));
        assert!(matching.contains(&"lib.rs".to_string()));
        assert!(!matching.contains(&"src/util.rs".to_string()));
    }

    #[test]
    fn index_search_stats_candidates_less_than_total() {
        let (dir, index_dir) = setup_corpus_and_index();
        let result = index_search(dir.path(), &index_dir, "MAX_FILE_SIZE").unwrap();

        // The index should filter: candidates <= total files
        assert!(result.stats.candidate_count <= result.stats.total_files as usize);
        assert!(result.stats.lookup_count > 0);
    }

    #[test]
    fn no_false_negatives_vs_scan() {
        let (dir, index_dir) = setup_corpus_and_index();
        let config = WalkConfig::default();

        let patterns = vec![
            "MAX_FILE_SIZE",
            "fn main",
            "parse_config",
            r"fn \w+\(\)",
            "helper",
            "NONEXISTENT",
        ];

        for pattern in patterns {
            let scan_files =
                qndx_core::scan::scan_matching_files(dir.path(), pattern, &config).unwrap();

            let reader = IndexReader::open(&index_dir).unwrap();
            let index_files = index_search_matching_files(&reader, dir.path(), pattern).unwrap();

            // Index must be a superset of scan (no false negatives)
            for f in &scan_files {
                assert!(
                    index_files.contains(f),
                    "FALSE NEGATIVE: pattern '{}', file '{}' found by scan but not index",
                    pattern,
                    f,
                );
            }
        }
    }
}
