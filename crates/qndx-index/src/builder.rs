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

use std::collections::BTreeMap;
use std::fs;
use std::io::BufWriter;
use std::path::Path;

use qndx_core::format::{
    self, encode_postings, serialize_ngram_entry, MAGIC_MANIFEST, MAGIC_NGRAMS, MAGIC_POSTINGS,
    NGRAM_ENTRY_SIZE,
};
use qndx_core::{FileId, Manifest, NgramEntry, NgramHash};

use crate::ngram::extract_trigrams;

/// Result of building an index.
#[derive(Debug)]
pub struct BuildResult {
    /// Number of files indexed.
    pub file_count: u32,
    /// Number of unique trigrams.
    pub ngram_count: u32,
    /// Total bytes of postings data.
    pub postings_bytes: u64,
    /// Total bytes of source files processed.
    pub source_bytes: u64,
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

    // Step 1: Build inverted index (trigram_hash -> sorted Vec<FileId>)
    let mut inverted: BTreeMap<NgramHash, Vec<FileId>> = BTreeMap::new();
    let mut source_bytes: u64 = 0;

    for (file_id, (_path, content)) in files.iter().enumerate() {
        source_bytes += content.len() as u64;
        let trigrams = extract_trigrams(content);
        for hash in trigrams {
            inverted
                .entry(hash)
                .or_default()
                .push(file_id as FileId);
        }
    }

    // Deduplicate postings (trigrams from same file should not duplicate the file_id)
    for posting in inverted.values_mut() {
        posting.sort_unstable();
        posting.dedup();
    }

    // Step 2: Serialize postings into a contiguous buffer
    let mut postings_payload = Vec::new();
    let mut ngram_entries: Vec<NgramEntry> = Vec::with_capacity(inverted.len());

    for (&hash, ids) in &inverted {
        let encoded = encode_postings(ids);
        let offset = postings_payload.len() as u64;
        let len = encoded.len() as u32;
        postings_payload.extend_from_slice(&encoded);

        ngram_entries.push(NgramEntry {
            hash,
            offset,
            len,
            flags: 0,
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

    let manifest_bytes = postcard::to_allocvec(&manifest)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    {
        let file = fs::File::create(index_dir.join("manifest.bin"))?;
        let mut writer = BufWriter::new(file);
        format::write_with_header(&mut writer, MAGIC_MANIFEST, &manifest_bytes)?;
    }

    Ok(BuildResult {
        file_count: files.len() as u32,
        ngram_count: ngram_entries.len() as u32,
        postings_bytes: postings_payload.len() as u64,
        source_bytes,
    })
}

/// Build a trigram index by walking a directory.
///
/// Discovers files using `WalkConfig`, reads them, and builds the index.
pub fn build_index_from_dir(
    root: &Path,
    index_dir: &Path,
    config: &qndx_core::walk::WalkConfig,
    base_commit: Option<String>,
) -> Result<BuildResult, format::FormatError> {
    let files = qndx_core::walk::discover_and_read_files(root, config);
    build_index(&files, index_dir, base_commit)
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let result = build_index(
            &sample_files(),
            &index_dir,
            Some("abc123".to_string()),
        )
        .unwrap();

        assert_eq!(result.file_count, 3);
    }

    #[test]
    fn build_single_tiny_file() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index/v1");
        let files = vec![("tiny.rs".to_string(), b"ab".to_vec())];

        let result = build_index(&files, &index_dir, None).unwrap();

        // "ab" has no trigrams
        assert_eq!(result.file_count, 1);
        assert_eq!(result.ngram_count, 0);
    }
}
