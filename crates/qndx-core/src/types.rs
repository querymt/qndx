//! Core types used across the qndx workspace.

use serde::{Deserialize, Serialize};

/// Unique identifier for a file in the index.
pub type FileId = u32;

/// An n-gram hash (used as key in the lookup table).
pub type NgramHash = u32;

/// Manifest metadata stored alongside the index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// Format version.
    pub version: u32,
    /// Number of indexed files.
    pub file_count: u32,
    /// Number of unique n-grams.
    pub ngram_count: u32,
    /// Total size of postings data in bytes.
    pub postings_bytes: u64,
    /// Git commit hash the index is based on (hex string).
    pub base_commit: Option<String>,
    /// File paths in index order.
    pub files: Vec<String>,
}

impl Manifest {
    pub fn new() -> Self {
        Self {
            version: 1,
            file_count: 0,
            ngram_count: 0,
            postings_bytes: 0,
            base_commit: None,
            files: Vec::new(),
        }
    }
}

impl Default for Manifest {
    fn default() -> Self {
        Self::new()
    }
}

/// Entry in the ngram lookup table (ngrams.tbl).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NgramEntry {
    /// Hash of the n-gram.
    pub hash: NgramHash,
    /// Byte offset into postings.dat.
    pub offset: u64,
    /// Length of the postings block in bytes.
    pub len: u32,
    /// Flags (reserved for future use).
    pub flags: u32,
}
