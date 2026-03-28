//! Index reader: load index files, binary search lookup, postings operations.
//!
//! Loads `ngrams.tbl`, `postings.dat`, and `manifest.bin` from an index directory
//! and provides efficient trigram -> candidate file set resolution.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use qndx_core::format::{
    self, decode_postings, deserialize_ngram_entry, FormatError, MAGIC_MANIFEST, MAGIC_NGRAMS,
    MAGIC_POSTINGS, NGRAM_ENTRY_SIZE,
};
use qndx_core::{FileId, Manifest, NgramEntry, NgramHash};

use crate::postings::PostingList;

/// A loaded trigram index, ready for queries.
#[derive(Debug)]
pub struct IndexReader {
    /// Sorted ngram table entries (sorted by hash).
    ngram_table: Vec<NgramEntry>,
    /// Raw postings data.
    postings_data: Vec<u8>,
    /// Manifest with metadata and file paths.
    pub manifest: Manifest,
}

impl IndexReader {
    /// Load an index from directory containing `ngrams.tbl`, `postings.dat`, `manifest.bin`.
    pub fn open(index_dir: &Path) -> Result<Self, FormatError> {
        // Load ngrams.tbl
        let ngrams_payload = {
            let file = File::open(index_dir.join("ngrams.tbl"))?;
            let mut reader = BufReader::new(file);
            format::read_with_header(&mut reader, MAGIC_NGRAMS)?
        };

        // Parse ngram entries
        let entry_count = ngrams_payload.len() / NGRAM_ENTRY_SIZE;
        let mut ngram_table = Vec::with_capacity(entry_count);
        for i in 0..entry_count {
            let start = i * NGRAM_ENTRY_SIZE;
            let buf: &[u8; NGRAM_ENTRY_SIZE] =
                ngrams_payload[start..start + NGRAM_ENTRY_SIZE].try_into().unwrap();
            ngram_table.push(deserialize_ngram_entry(buf));
        }

        // Load postings.dat
        let postings_data = {
            let file = File::open(index_dir.join("postings.dat"))?;
            let mut reader = BufReader::new(file);
            format::read_with_header(&mut reader, MAGIC_POSTINGS)?
        };

        // Load manifest.bin
        let manifest_bytes = {
            let file = File::open(index_dir.join("manifest.bin"))?;
            let mut reader = BufReader::new(file);
            format::read_with_header(&mut reader, MAGIC_MANIFEST)?
        };

        let manifest: Manifest = postcard::from_bytes(&manifest_bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        Ok(IndexReader {
            ngram_table,
            postings_data,
            manifest,
        })
    }

    /// Look up a single trigram hash via binary search.
    /// Returns the posting list for that trigram, or an empty list if not found.
    pub fn lookup(&self, hash: NgramHash) -> PostingList {
        match self
            .ngram_table
            .binary_search_by_key(&hash, |entry| entry.hash)
        {
            Ok(idx) => {
                let entry = &self.ngram_table[idx];
                let start = entry.offset as usize;
                let end = start + entry.len as usize;
                if end <= self.postings_data.len() {
                    let ids = decode_postings(&self.postings_data[start..end]);
                    PostingList::from_vec(ids)
                } else {
                    PostingList::from_vec(vec![])
                }
            }
            Err(_) => PostingList::from_vec(vec![]),
        }
    }

    /// Look up multiple trigram hashes and intersect their posting lists (AND semantics).
    /// Returns the set of FileIds that contain ALL given trigrams.
    /// If `hashes` is empty, returns all file IDs (no filtering).
    pub fn lookup_intersect(&self, hashes: &[NgramHash]) -> PostingList {
        if hashes.is_empty() {
            // No trigrams to filter on: all files are candidates
            let all: Vec<FileId> = (0..self.manifest.file_count).collect();
            return PostingList::from_vec(all);
        }

        let mut iter = hashes.iter();
        let first = *iter.next().unwrap();
        let mut result = self.lookup(first);

        for &hash in iter {
            if result.is_empty() {
                break; // Short-circuit: intersection with empty is empty
            }
            let posting = self.lookup(hash);
            result = result.intersect(&posting);
        }

        result
    }

    /// Look up multiple trigram hashes and union their posting lists (OR semantics).
    /// Returns the set of FileIds that contain ANY of the given trigrams.
    pub fn lookup_union(&self, hashes: &[NgramHash]) -> PostingList {
        if hashes.is_empty() {
            return PostingList::from_vec(vec![]);
        }

        let mut iter = hashes.iter();
        let first = *iter.next().unwrap();
        let mut result = self.lookup(first);

        for &hash in iter {
            let posting = self.lookup(hash);
            result = result.union(&posting);
        }

        result
    }

    /// Resolve a FileId to its file path.
    pub fn file_path(&self, id: FileId) -> Option<&str> {
        self.manifest.files.get(id as usize).map(|s| s.as_str())
    }

    /// Get the number of unique trigrams in the index.
    pub fn ngram_count(&self) -> usize {
        self.ngram_table.len()
    }

    /// Get the number of indexed files.
    pub fn file_count(&self) -> u32 {
        self.manifest.file_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::build_index;
    use qndx_core::hash_ngram;

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

    fn build_and_open(files: &[(String, Vec<u8>)]) -> IndexReader {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index/v1");
        build_index(files, &index_dir, None).unwrap();
        // We need to keep the tempdir alive, so leak it for tests
        let index_dir_owned = index_dir.clone();
        std::mem::forget(dir);
        IndexReader::open(&index_dir_owned).unwrap()
    }

    #[test]
    fn roundtrip_build_and_open() {
        let files = sample_files();
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index/v1");
        build_index(&files, &index_dir, None).unwrap();

        let reader = IndexReader::open(&index_dir).unwrap();
        assert_eq!(reader.file_count(), 3);
        assert!(reader.ngram_count() > 0);
        assert_eq!(reader.file_path(0), Some("main.rs"));
        assert_eq!(reader.file_path(1), Some("lib.rs"));
        assert_eq!(reader.file_path(2), Some("util.rs"));
    }

    #[test]
    fn lookup_known_trigram() {
        let reader = build_and_open(&sample_files());

        // "MAX" trigram should be found in files containing "MAX_FILE_SIZE"
        let hash = hash_ngram(b"MAX");
        let posting = reader.lookup(hash);
        let ids = posting.to_vec();

        // main.rs (0) and lib.rs (1) contain MAX_FILE_SIZE
        assert!(ids.contains(&0), "main.rs should contain MAX trigram");
        assert!(ids.contains(&1), "lib.rs should contain MAX trigram");
    }

    #[test]
    fn lookup_missing_trigram() {
        let reader = build_and_open(&sample_files());

        let hash = hash_ngram(b"ZZZ");
        let posting = reader.lookup(hash);
        assert!(posting.is_empty());
    }

    #[test]
    fn intersect_narrows_candidates() {
        let reader = build_and_open(&sample_files());

        // Trigrams from "MAX_FILE_SIZE" should narrow to files 0 and 1
        let hashes: Vec<NgramHash> = [b"MAX" as &[u8], b"_FI", b"ILE"]
            .iter()
            .map(|t| hash_ngram(t))
            .collect();

        let result = reader.lookup_intersect(&hashes);
        let ids = result.to_vec();

        assert!(ids.contains(&0)); // main.rs
        assert!(ids.contains(&1)); // lib.rs
        assert!(!ids.contains(&2)); // util.rs should NOT be included
    }

    #[test]
    fn union_expands_candidates() {
        let reader = build_and_open(&sample_files());

        // "hel" is in util.rs (helper), "MAX" is in main.rs and lib.rs
        let hashes: Vec<NgramHash> = [b"hel" as &[u8], b"MAX"]
            .iter()
            .map(|t| hash_ngram(t))
            .collect();

        let result = reader.lookup_union(&hashes);
        let ids = result.to_vec();

        assert!(ids.len() >= 2); // at least util.rs + one of main/lib
    }

    #[test]
    fn empty_hashes_intersect_returns_all() {
        let reader = build_and_open(&sample_files());

        let result = reader.lookup_intersect(&[]);
        assert_eq!(result.to_vec().len(), 3);
    }

    #[test]
    fn empty_hashes_union_returns_empty() {
        let reader = build_and_open(&sample_files());

        let result = reader.lookup_union(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn rejects_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index/v1");
        build_index(&sample_files(), &index_dir, None).unwrap();

        // Corrupt the ngrams file
        let ngrams_path = index_dir.join("ngrams.tbl");
        let mut data = std::fs::read(&ngrams_path).unwrap();
        if data.len() > 25 {
            data[25] ^= 0xFF;
        }
        std::fs::write(&ngrams_path, &data).unwrap();

        let result = IndexReader::open(&index_dir);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_wrong_magic() {
        let dir = tempfile::tempdir().unwrap();
        let index_dir = dir.path().join("index/v1");
        build_index(&sample_files(), &index_dir, None).unwrap();

        // Overwrite magic bytes in ngrams.tbl
        let ngrams_path = index_dir.join("ngrams.tbl");
        let mut data = std::fs::read(&ngrams_path).unwrap();
        data[0..4].copy_from_slice(b"XXXX");
        std::fs::write(&ngrams_path, &data).unwrap();

        let result = IndexReader::open(&index_dir);
        assert!(result.is_err());
    }
}
