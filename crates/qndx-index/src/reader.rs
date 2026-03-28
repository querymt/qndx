//! Index reader: memory-mapped index files, binary search lookup, postings operations.
//!
//! Uses `memmap2` to memory-map `ngrams.tbl` and `postings.dat`, avoiding the need
//! to read entire files into heap memory. The OS manages page-in/page-out, so only
//! pages actually touched during queries consume physical RAM.
//!
//! For the Linux kernel index (~2 GB on disk), this reduces query-time resident memory
//! from ~2 GB to effectively zero upfront cost (only touched pages are paged in).

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use memmap2::Mmap;
use qndx_core::format::{
    self, deserialize_ngram_entry, payload_from_slice, validate_checksum_from_slice,
    validate_header_from_slice, FormatError, FLAG_SPARSE, MAGIC_MANIFEST, MAGIC_NGRAMS,
    MAGIC_POSTINGS, NGRAM_ENTRY_SIZE,
};
use qndx_core::{FileId, Manifest, NgramHash};

use crate::postings::PostingList;

/// A memory-mapped trigram index, ready for queries.
///
/// The ngram table and postings data are memory-mapped from disk. Only the manifest
/// (file paths and metadata) is fully deserialized into heap memory.
pub struct IndexReader {
    /// Memory-mapped ngrams.tbl payload region (after header).
    ngram_mmap: Mmap,
    /// Memory-mapped postings.dat payload region (after header).
    postings_mmap: Mmap,
    /// Number of ngram entries (computed from mmap size).
    ngram_count: usize,
    /// Manifest with metadata and file paths (deserialized, typically small).
    pub manifest: Manifest,
}

// Manual Debug impl because Mmap doesn't implement Debug
impl std::fmt::Debug for IndexReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexReader")
            .field("ngram_count", &self.ngram_count)
            .field("ngram_mmap_len", &self.ngram_mmap.len())
            .field("postings_mmap_len", &self.postings_mmap.len())
            .field("manifest.file_count", &self.manifest.file_count)
            .finish()
    }
}

impl IndexReader {
    /// Load an index from directory containing `ngrams.tbl`, `postings.dat`, `manifest.bin`.
    ///
    /// Uses memory-mapped I/O for ngrams and postings files. The manifest is read
    /// fully into memory (it's typically small — a few KB even for large corpora).
    pub fn open(index_dir: &Path) -> Result<Self, FormatError> {
        // Memory-map ngrams.tbl
        let ngram_mmap = {
            let file = File::open(index_dir.join("ngrams.tbl"))?;
            // SAFETY: the file is read-only and we don't modify it.
            let mmap = unsafe { Mmap::map(&file)? };
            let header = validate_header_from_slice(&mmap, MAGIC_NGRAMS)?;
            validate_checksum_from_slice(&mmap, &header)?;
            mmap
        };

        let ngram_payload = payload_from_slice(&ngram_mmap);
        let ngram_count = ngram_payload.len() / NGRAM_ENTRY_SIZE;

        // Memory-map postings.dat
        let postings_mmap = {
            let file = File::open(index_dir.join("postings.dat"))?;
            // SAFETY: the file is read-only and we don't modify it.
            let mmap = unsafe { Mmap::map(&file)? };
            let header = validate_header_from_slice(&mmap, MAGIC_POSTINGS)?;
            validate_checksum_from_slice(&mmap, &header)?;
            mmap
        };

        // Load manifest.bin (small, fully deserialized)
        let manifest_bytes = {
            let file = File::open(index_dir.join("manifest.bin"))?;
            let mut reader = BufReader::new(file);
            format::read_with_header(&mut reader, MAGIC_MANIFEST)?
        };

        let manifest: Manifest = postcard::from_bytes(&manifest_bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        Ok(IndexReader {
            ngram_mmap,
            postings_mmap,
            ngram_count,
            manifest,
        })
    }

    /// Get the ngram payload region (after the file header).
    #[inline]
    fn ngram_payload(&self) -> &[u8] {
        payload_from_slice(&self.ngram_mmap)
    }

    /// Get the postings payload region (after the file header).
    #[inline]
    fn postings_payload(&self) -> &[u8] {
        payload_from_slice(&self.postings_mmap)
    }

    /// Deserialize a single ngram entry at the given index from the mmap'd table.
    #[inline]
    fn ngram_entry(&self, idx: usize) -> qndx_core::NgramEntry {
        let payload = self.ngram_payload();
        let start = idx * NGRAM_ENTRY_SIZE;
        let buf: &[u8; NGRAM_ENTRY_SIZE] =
            payload[start..start + NGRAM_ENTRY_SIZE].try_into().unwrap();
        deserialize_ngram_entry(buf)
    }

    /// Read just the hash field from an ngram entry at the given index.
    /// Avoids deserializing the full entry during binary search.
    #[inline]
    fn ngram_hash_at(&self, idx: usize) -> NgramHash {
        let payload = self.ngram_payload();
        let start = idx * NGRAM_ENTRY_SIZE;
        u32::from_le_bytes(payload[start..start + 4].try_into().unwrap())
    }

    /// Binary search the ngram table for a hash.
    /// Returns the index into the ngram table, or None if not found.
    fn binary_search_hash(&self, hash: NgramHash) -> Option<usize> {
        let mut lo = 0usize;
        let mut hi = self.ngram_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let mid_hash = self.ngram_hash_at(mid);
            match mid_hash.cmp(&hash) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid),
            }
        }
        None
    }

    /// Look up a single trigram hash via binary search.
    /// Returns the posting list for that trigram, or an empty list if not found.
    ///
    /// The on-disk postings block is prefixed with a 1-byte tag that identifies
    /// the encoding format (varint-delta, fixed-delta, or Roaring). The reader
    /// auto-detects the format using `PostingList::decode_tagged`.
    pub fn lookup(&self, hash: NgramHash) -> PostingList {
        match self.binary_search_hash(hash) {
            Some(idx) => {
                let entry = self.ngram_entry(idx);
                let postings = self.postings_payload();
                let start = entry.offset as usize;
                let end = start + entry.len as usize;
                if end <= postings.len() {
                    PostingList::decode_tagged(&postings[start..end])
                        .unwrap_or_else(|| PostingList::Vec(vec![]))
                } else {
                    PostingList::Vec(vec![])
                }
            }
            None => PostingList::Vec(vec![]),
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

    /// Get the number of unique n-grams in the index (trigrams + sparse).
    pub fn ngram_count(&self) -> usize {
        self.ngram_count
    }

    /// Get the number of sparse n-gram entries in the index.
    pub fn sparse_count(&self) -> usize {
        (0..self.ngram_count)
            .filter(|&i| self.ngram_entry(i).flags & FLAG_SPARSE != 0)
            .count()
    }

    /// Get the number of trigram-only entries in the index.
    pub fn trigram_only_count(&self) -> usize {
        (0..self.ngram_count)
            .filter(|&i| self.ngram_entry(i).flags & FLAG_SPARSE == 0)
            .count()
    }

    /// Check if a given n-gram hash exists in the index.
    pub fn contains(&self, hash: NgramHash) -> bool {
        self.binary_search_hash(hash).is_some()
    }

    /// Check if a given n-gram hash is a sparse n-gram.
    pub fn is_sparse(&self, hash: NgramHash) -> bool {
        match self.binary_search_hash(hash) {
            Some(idx) => self.ngram_entry(idx).flags & FLAG_SPARSE != 0,
            None => false,
        }
    }

    /// Get the posting list length (document frequency) for a given n-gram hash.
    /// Returns 0 if the hash is not found.
    pub fn posting_len(&self, hash: NgramHash) -> usize {
        self.lookup(hash).len()
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
