//! Index builder: extract trigrams from files, write ngrams.tbl + postings.dat + manifest.bin.
//!
//! The build pipeline:
//! 1. Walk files and assign sequential FileIds
//! 2. Extract overlapping trigrams from each file
//! 3. Collect inverted index: trigram_hash -> Vec<FileId>
//! 4. Sort trigram table by hash for binary search
//! 5. Write postings.dat (concatenated delta-encoded posting blocks)
//! 6. Write ngrams.tbl (sorted hash -> offset/len/flags entries)
//! 7. Write manifest.bin (metadata + file path list)

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use qndx_core::format::{
    self, FLAG_SPARSE, MAGIC_MANIFEST, MAGIC_NGRAMS, MAGIC_POSTINGS, NGRAM_ENTRY_SIZE,
    serialize_ngram_entry,
};
use qndx_core::{FileId, Manifest, NgramEntry, NgramHash};

use crate::ngram::{extract_sparse_ngrams_all, extract_trigrams};
use crate::postings::{DEFAULT_HYBRID_THRESHOLD, PostingList};
use crate::reader::IndexReader;

/// Result of building an index.
#[derive(Debug)]
pub struct BuildResult {
    /// Number of files indexed.
    pub file_count: u32,
    /// Number of unique n-grams (trigrams + sparse).
    pub ngram_count: u32,
    /// Number of trigram-only entries.
    pub trigram_count: u32,
    /// Number of sparse n-gram entries.
    pub sparse_count: u32,
    /// Total bytes of postings data.
    pub postings_bytes: u64,
    /// Total bytes of source files processed.
    pub source_bytes: u64,
}

/// Result of an incremental update attempt.
#[derive(Debug)]
pub struct IncrementalResult {
    /// True when no files changed since base commit.
    pub up_to_date: bool,
    /// Number of files changed since base commit.
    pub changed_files: usize,
    /// Number of indexed files before rebuild.
    pub previous_file_count: u32,
    /// Number of indexed files after rebuild.
    pub new_file_count: u32,
    /// Whether we fell back to a full rebuild due to change ratio.
    pub forced_full_rebuild: bool,
    /// Build stats when a rebuild happened.
    pub build_result: Option<BuildResult>,
}

/// Build a trigram index from in-memory file data.
///
/// `files` is a list of (relative_path, content) pairs.
/// Writes `ngrams.tbl`, `postings.dat`, and `manifest.bin` into `index_dir`.
pub fn build_index(
    files: &[(String, Vec<u8>)],
    index_dir: &Path,
    base_commit: Option<String>,
) -> Result<BuildResult, format::FormatError> {
    fs::create_dir_all(index_dir)?;

    // Step 1: Build inverted index (ngram_hash -> sorted Vec<FileId>)
    // We track which hashes are sparse vs trigram via a separate set.
    let mut inverted: BTreeMap<NgramHash, Vec<FileId>> = BTreeMap::new();
    let mut sparse_hashes: HashSet<NgramHash> = HashSet::new();
    let mut source_bytes: u64 = 0;

    for (file_id, (_path, content)) in files.iter().enumerate() {
        source_bytes += content.len() as u64;
        let fid = file_id as FileId;

        // Extract trigrams (baseline)
        let trigrams = extract_trigrams(content);
        for hash in trigrams {
            inverted.entry(hash).or_default().push(fid);
        }

        // Extract sparse n-grams (build-all approach)
        let sparse = extract_sparse_ngrams_all(content);
        for (hash, _len) in sparse {
            sparse_hashes.insert(hash);
            inverted.entry(hash).or_default().push(fid);
        }
    }

    // Deduplicate postings (same file should not appear twice for same n-gram)
    for posting in inverted.values_mut() {
        posting.sort_unstable();
        posting.dedup();
    }

    // Step 2: Serialize postings into a contiguous buffer using tagged hybrid format.
    // Each posting block is prefixed with a 1-byte tag so the reader can auto-detect
    // whether it was stored as varint-delta (small lists) or Roaring (large lists).
    let mut postings_payload = Vec::new();
    let mut ngram_entries: Vec<NgramEntry> = Vec::with_capacity(inverted.len());
    let mut trigram_count: u32 = 0;
    let mut sparse_count: u32 = 0;

    for (&hash, ids) in &inverted {
        let posting = PostingList::from_vec_with_threshold(ids.clone(), DEFAULT_HYBRID_THRESHOLD);
        let encoded = posting.encode_auto();
        let offset = postings_payload.len() as u64;
        let len = encoded.len() as u32;
        postings_payload.extend_from_slice(&encoded);

        let flags = if sparse_hashes.contains(&hash) {
            sparse_count += 1;
            FLAG_SPARSE
        } else {
            trigram_count += 1;
            0
        };

        ngram_entries.push(NgramEntry {
            hash,
            offset,
            len,
            flags,
        });
    }

    // Step 3: Serialize ngram table (already sorted since BTreeMap iterates in order)
    let mut ngrams_payload = Vec::with_capacity(ngram_entries.len() * NGRAM_ENTRY_SIZE);
    for entry in &ngram_entries {
        ngrams_payload.extend_from_slice(&serialize_ngram_entry(entry));
    }

    // Step 4: Write ngrams.tbl
    {
        let file = fs::File::create(index_dir.join("ngrams.tbl"))?;
        let mut writer = BufWriter::new(file);
        format::write_with_header(&mut writer, MAGIC_NGRAMS, &ngrams_payload)?;
    }

    // Step 5: Write postings.dat
    {
        let file = fs::File::create(index_dir.join("postings.dat"))?;
        let mut writer = BufWriter::new(file);
        format::write_with_header(&mut writer, MAGIC_POSTINGS, &postings_payload)?;
    }

    // Step 6: Write manifest.bin
    let manifest = Manifest {
        version: qndx_core::format::FORMAT_VERSION,
        file_count: files.len() as u32,
        ngram_count: ngram_entries.len() as u32,
        postings_bytes: postings_payload.len() as u64,
        base_commit,
        files: files.iter().map(|(path, _)| path.clone()).collect(),
    };

    let manifest_bytes =
        postcard::to_allocvec(&manifest).map_err(|e| std::io::Error::other(e.to_string()))?;

    {
        let file = fs::File::create(index_dir.join("manifest.bin"))?;
        let mut writer = BufWriter::new(file);
        format::write_with_header(&mut writer, MAGIC_MANIFEST, &manifest_bytes)?;
    }

    Ok(BuildResult {
        file_count: files.len() as u32,
        ngram_count: ngram_entries.len() as u32,
        trigram_count,
        sparse_count,
        postings_bytes: postings_payload.len() as u64,
        source_bytes,
    })
}

/// Build a trigram index by walking a directory.
///
/// Discovers files using `WalkConfig` and processes them one at a time,
/// avoiding loading the entire corpus into memory. Only the inverted index
/// (n-gram → file IDs) is held in memory during the build.
///
/// For a 1 GB corpus this reduces peak memory from ~1 GB (all file content)
/// + inverted index to just the inverted index + O(largest_file).
pub fn build_index_from_dir(
    root: &Path,
    index_dir: &Path,
    config: &qndx_core::walk::WalkConfig,
    base_commit: Option<String>,
) -> Result<BuildResult, format::FormatError> {
    fs::create_dir_all(index_dir)?;

    let discovered = qndx_core::walk::discover_files(root, config);

    // Step 1: Build inverted index by streaming files one at a time
    let mut inverted: BTreeMap<NgramHash, Vec<FileId>> = BTreeMap::new();
    let mut sparse_hashes: HashSet<NgramHash> = HashSet::new();
    let mut source_bytes: u64 = 0;
    let mut file_paths: Vec<String> = Vec::with_capacity(discovered.len());

    for file in &discovered {
        let content = match std::fs::read(&file.abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let fid = file_paths.len() as FileId;
        file_paths.push(file.rel_path.clone());
        source_bytes += content.len() as u64;

        // Extract trigrams
        let trigrams = extract_trigrams(&content);
        for hash in trigrams {
            inverted.entry(hash).or_default().push(fid);
        }

        // Extract sparse n-grams (build-all approach)
        let sparse = extract_sparse_ngrams_all(&content);
        for (hash, _len) in sparse {
            sparse_hashes.insert(hash);
            inverted.entry(hash).or_default().push(fid);
        }
        // `content` is dropped here — only one file in memory at a time
    }

    // Deduplicate postings
    for posting in inverted.values_mut() {
        posting.sort_unstable();
        posting.dedup();
    }

    // Step 2–6: Serialize and write (same as build_index)
    let mut postings_payload = Vec::new();
    let mut ngram_entries: Vec<NgramEntry> = Vec::with_capacity(inverted.len());
    let mut trigram_count: u32 = 0;
    let mut sparse_count: u32 = 0;

    for (&hash, ids) in &inverted {
        let posting = PostingList::from_vec_with_threshold(ids.clone(), DEFAULT_HYBRID_THRESHOLD);
        let encoded = posting.encode_auto();
        let offset = postings_payload.len() as u64;
        let len = encoded.len() as u32;
        postings_payload.extend_from_slice(&encoded);

        let flags = if sparse_hashes.contains(&hash) {
            sparse_count += 1;
            FLAG_SPARSE
        } else {
            trigram_count += 1;
            0
        };

        ngram_entries.push(NgramEntry {
            hash,
            offset,
            len,
            flags,
        });
    }

    // Drop the inverted index — no longer needed
    drop(inverted);
    drop(sparse_hashes);

    let mut ngrams_payload = Vec::with_capacity(ngram_entries.len() * NGRAM_ENTRY_SIZE);
    for entry in &ngram_entries {
        ngrams_payload.extend_from_slice(&serialize_ngram_entry(entry));
    }

    {
        let file = fs::File::create(index_dir.join("ngrams.tbl"))?;
        let mut writer = std::io::BufWriter::new(file);
        format::write_with_header(&mut writer, MAGIC_NGRAMS, &ngrams_payload)?;
    }

    {
        let file = fs::File::create(index_dir.join("postings.dat"))?;
        let mut writer = std::io::BufWriter::new(file);
        format::write_with_header(&mut writer, MAGIC_POSTINGS, &postings_payload)?;
    }

    let file_count = file_paths.len() as u32;
    let manifest = Manifest {
        version: qndx_core::format::FORMAT_VERSION,
        file_count,
        ngram_count: ngram_entries.len() as u32,
        postings_bytes: postings_payload.len() as u64,
        base_commit,
        files: file_paths,
    };

    let manifest_bytes =
        postcard::to_allocvec(&manifest).map_err(|e| std::io::Error::other(e.to_string()))?;

    {
        let file = fs::File::create(index_dir.join("manifest.bin"))?;
        let mut writer = std::io::BufWriter::new(file);
        format::write_with_header(&mut writer, MAGIC_MANIFEST, &manifest_bytes)?;
    }

    Ok(BuildResult {
        file_count,
        ngram_count: ngram_entries.len() as u32,
        trigram_count,
        sparse_count,
        postings_bytes: postings_payload.len() as u64,
        source_bytes,
    })
}

/// Update an existing index using git change detection and smart skip behavior.
///
/// Current implementation performs a full rebuild when changes are detected,
/// but can skip work entirely when the index is already up to date.
pub fn update_index_from_dir(
    root: &Path,
    index_dir: &Path,
    config: &qndx_core::walk::WalkConfig,
    new_base_commit: Option<String>,
    change_threshold_percent: u8,
) -> Result<IncrementalResult, format::FormatError> {
    let reader = IndexReader::open(index_dir)?;
    let previous_file_count = reader.file_count();
    let base_commit = reader.manifest.base_commit.clone();
    drop(reader);

    let root_abs = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let index_dir_abs = index_dir.canonicalize().ok();

    let rebuild = |forced_full_rebuild: bool| {
        let build_result = build_index_from_dir(root, index_dir, config, new_base_commit.clone())?;
        Ok::<IncrementalResult, format::FormatError>(IncrementalResult {
            up_to_date: false,
            changed_files: 0,
            previous_file_count,
            new_file_count: build_result.file_count,
            forced_full_rebuild,
            build_result: Some(build_result),
        })
    };

    let Some(base_commit) = base_commit else {
        return rebuild(true);
    };

    let repo = match qndx_git::GitRepo::open(root) {
        Ok(repo) => repo,
        Err(_) => return rebuild(true),
    };

    let changes = match repo.detect_changes_since(&base_commit) {
        Ok(changes) => changes,
        Err(_) => return rebuild(true),
    };

    let mut latest_changes: HashMap<PathBuf, qndx_git::FileStatus> = HashMap::new();
    for (path, status) in changes {
        let abs_path = root_abs.join(&path);
        if let Some(idx_abs) = &index_dir_abs
            && abs_path.starts_with(idx_abs)
        {
            continue;
        }
        latest_changes.insert(path, status);
    }

    let changed_files = latest_changes
        .values()
        .filter(|status| !matches!(status, qndx_git::FileStatus::Clean))
        .count();

    if changed_files == 0 {
        return Ok(IncrementalResult {
            up_to_date: true,
            changed_files: 0,
            previous_file_count,
            new_file_count: previous_file_count,
            forced_full_rebuild: false,
            build_result: None,
        });
    }

    let change_ratio = if previous_file_count == 0 {
        100.0
    } else {
        (changed_files as f64 * 100.0) / previous_file_count as f64
    };

    let build_result = build_index_from_dir(root, index_dir, config, new_base_commit)?;
    Ok(IncrementalResult {
        up_to_date: false,
        changed_files,
        previous_file_count,
        new_file_count: build_result.file_count,
        forced_full_rebuild: change_ratio > change_threshold_percent as f64,
        build_result: Some(build_result),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn sample_files() -> Vec<(String, Vec<u8>)> {
        vec![
            (
                "main.rs".to_string(),
                b"fn main() {\n    let x = MAX_FILE_SIZE;\n}\n".to_vec(),
            ),
            (
                "lib.rs".to_string(),
                b"pub const MAX_FILE_SIZE: usize = 1024;\n".to_vec(),
            ),
            (
                "util.rs".to_string(),
                b"fn helper() -> u32 { 42 }\n".to_vec(),
            ),
        ]
    }

    #[test]
    fn build_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index/v1");

        let result = build_index(&sample_files(), &index_dir, None).unwrap();

        assert_eq!(result.file_count, 3);
        assert!(result.ngram_count > 0);
        assert!(result.postings_bytes > 0);
        assert!(index_dir.join("ngrams.tbl").exists());
        assert!(index_dir.join("postings.dat").exists());
        assert!(index_dir.join("manifest.bin").exists());
    }

    #[test]
    fn build_empty_corpus() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index/v1");
        let files: Vec<(String, Vec<u8>)> = vec![];

        let result = build_index(&files, &index_dir, None).unwrap();

        assert_eq!(result.file_count, 0);
        assert_eq!(result.ngram_count, 0);
        assert_eq!(result.postings_bytes, 0);
    }

    #[test]
    fn build_with_base_commit() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index/v1");

        let result = build_index(&sample_files(), &index_dir, Some("abc123".to_string())).unwrap();

        assert_eq!(result.file_count, 3);
    }

    #[test]
    fn build_single_tiny_file() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index/v1");
        let files = vec![("tiny.rs".to_string(), b"ab".to_vec())];

        let result = build_index(&files, &index_dir, None).unwrap();

        // "ab" has no trigrams but produces a sparse bigram
        assert_eq!(result.file_count, 1);
        assert_eq!(result.trigram_count, 0);
        assert!(result.ngram_count >= result.sparse_count);
    }

    #[test]
    fn build_includes_sparse_ngrams() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index/v1");

        let result = build_index(&sample_files(), &index_dir, None).unwrap();

        // build_all produces all qualifying substrings, including bigram and
        // trigram-length spans. Hashes that appear in both trigram and sparse
        // extraction are classified as sparse (FLAG_SPARSE wins). So
        // trigram_count may be 0 if build_all covers all trigram hashes.
        assert!(result.ngram_count > 0, "should have n-grams");
        assert!(result.sparse_count > 0, "should have sparse n-grams");
        assert_eq!(
            result.ngram_count,
            result.trigram_count + result.sparse_count,
            "total should equal trigram-only + sparse"
        );
    }

    fn setup_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        Command::new("git")
            .args(["init"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(root)
            .output()
            .unwrap();

        fs::write(root.join("main.rs"), "fn main() { println!(\"old\"); }\n").unwrap();
        fs::write(root.join("lib.rs"), "pub const VERSION: &str = \"1\";\n").unwrap();

        Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(root)
            .output()
            .unwrap();

        dir
    }

    #[test]
    fn incremental_up_to_date_skips_rebuild() {
        let dir = setup_git_repo();
        let root = dir.path();
        let index_dir = root.join(".qndx/index/v1");
        let config = qndx_core::walk::WalkConfig::default();

        let base = qndx_git::head_commit(root).unwrap();
        build_index_from_dir(root, &index_dir, &config, Some(base.clone())).unwrap();

        let result = update_index_from_dir(root, &index_dir, &config, Some(base), 50).unwrap();
        assert!(result.up_to_date);
        assert_eq!(result.changed_files, 0);
        assert!(result.build_result.is_none());
    }

    #[test]
    fn incremental_rebuild_when_changed() {
        let dir = setup_git_repo();
        let root = dir.path();
        let index_dir = root.join(".qndx/index/v1");
        let config = qndx_core::walk::WalkConfig::default();

        let base = qndx_git::head_commit(root).unwrap();
        build_index_from_dir(root, &index_dir, &config, Some(base)).unwrap();

        fs::write(root.join("main.rs"), "fn main() { println!(\"new\"); }\n").unwrap();

        let new_head = qndx_git::head_commit(root).ok();
        let result = update_index_from_dir(root, &index_dir, &config, new_head, 50).unwrap();

        assert!(!result.up_to_date);
        assert!(result.changed_files >= 1);
        assert!(result.build_result.is_some());
    }
}
