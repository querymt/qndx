//! Working-tree overlay: a lightweight mini-index for dirty files.
//!
//! This module provides fast incremental updates for modified/added/deleted files
//! without requiring a full reindex. The overlay sits on top of the baseline index
//! and provides read-your-writes semantics.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use qndx_core::{FileId, NgramHash};
use qndx_git::{FileStatus, GitRepo};

use crate::ngram::{extract_sparse_ngrams_all, extract_trigrams};
use crate::postings::PostingList;

/// A lightweight overlay index for dirty files in the working tree.
///
/// The overlay tracks:
/// - Modified files: re-indexed content
/// - Added files: newly indexed content
/// - Deleted files: marked for exclusion from baseline results
#[derive(Debug)]
pub struct OverlayIndex {
    /// Inverted index: ngram_hash -> set of overlay file IDs.
    /// File IDs start from a high offset to avoid collision with baseline IDs.
    ngrams: HashMap<NgramHash, Vec<FileId>>,

    /// Map from overlay file ID to relative path.
    files: Vec<PathBuf>,

    /// Map from relative path to overlay file ID (for quick lookups).
    path_to_id: HashMap<PathBuf, FileId>,

    /// Set of file paths that have been deleted (to exclude from baseline).
    deleted: HashSet<PathBuf>,

    /// Starting file ID offset (to avoid collision with baseline index).
    /// We use high file IDs (e.g., starting from 1_000_000_000).
    base_file_id: FileId,
}

impl OverlayIndex {
    /// Create a new empty overlay index.
    ///
    /// `base_file_id` is the starting ID for overlay files (should be higher than
    /// any baseline index file ID to avoid collisions).
    pub fn new(base_file_id: FileId) -> Self {
        Self {
            ngrams: HashMap::new(),
            files: Vec::new(),
            path_to_id: HashMap::new(),
            deleted: HashSet::new(),
            base_file_id,
        }
    }

    /// Build an overlay from dirty files detected by Git.
    ///
    /// `repo_root` is the repository root directory.
    /// `dirty_files` is the list of (relative_path, status) pairs from Git.
    pub fn from_dirty_files(
        repo_root: &Path,
        dirty_files: &[(PathBuf, FileStatus)],
        base_file_id: FileId,
    ) -> Result<Self, std::io::Error> {
        let mut overlay = Self::new(base_file_id);

        for (rel_path, status) in dirty_files {
            match status {
                FileStatus::Modified | FileStatus::Added => {
                    // Read and index the file
                    let abs_path = repo_root.join(rel_path);
                    if let Ok(content) = std::fs::read(&abs_path) {
                        overlay.add_file(rel_path.clone(), &content);
                    }
                }
                FileStatus::Deleted => {
                    // Mark as deleted (exclude from baseline results)
                    overlay.mark_deleted(rel_path.clone());
                }
                FileStatus::Clean => {
                    // Should not appear in dirty files list
                }
            }
        }

        Ok(overlay)
    }

    /// Detect dirty files using Git and build an overlay.
    pub fn from_git_repo(
        repo: &GitRepo,
        base_file_id: FileId,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let repo_root = repo
            .root_path()
            .ok_or("repository has no working directory")?;

        let dirty_files = repo.detect_dirty_files()?;

        Ok(Self::from_dirty_files(
            repo_root,
            &dirty_files,
            base_file_id,
        )?)
    }

    /// Add or update a file in the overlay.
    fn add_file(&mut self, rel_path: PathBuf, content: &[u8]) {
        // Assign a new file ID or reuse existing one
        let file_id = if let Some(&existing_id) = self.path_to_id.get(&rel_path) {
            // File already in overlay: remove old n-gram entries
            self.remove_file_ngrams(existing_id);
            existing_id
        } else {
            // New file: assign next ID
            let new_id = self.base_file_id + self.files.len() as FileId;
            self.files.push(rel_path.clone());
            self.path_to_id.insert(rel_path.clone(), new_id);
            new_id
        };

        // Also remove from deleted set if it was there
        self.deleted.remove(&rel_path);

        // Extract trigrams
        let trigrams = extract_trigrams(content);
        for hash in trigrams {
            self.ngrams.entry(hash).or_default().push(file_id);
        }

        // Extract sparse n-grams (build-all approach)
        let sparse = extract_sparse_ngrams_all(content);
        for (hash, _len) in sparse {
            self.ngrams.entry(hash).or_default().push(file_id);
        }

        // Deduplicate postings for this file
        for posting in self.ngrams.values_mut() {
            posting.sort_unstable();
            posting.dedup();
        }
    }

    /// Mark a file as deleted.
    fn mark_deleted(&mut self, rel_path: PathBuf) {
        self.deleted.insert(rel_path.clone());

        // If the file was in the overlay, remove it
        if let Some(&file_id) = self.path_to_id.get(&rel_path) {
            self.remove_file_ngrams(file_id);
            self.path_to_id.remove(&rel_path);
        }
    }

    /// Remove all n-gram entries for a given file ID.
    fn remove_file_ngrams(&mut self, file_id: FileId) {
        for posting in self.ngrams.values_mut() {
            posting.retain(|&id| id != file_id);
        }

        // Clean up empty postings
        self.ngrams.retain(|_, posting| !posting.is_empty());
    }

    /// Look up an n-gram in the overlay index.
    pub fn lookup(&self, hash: NgramHash) -> PostingList {
        self.ngrams
            .get(&hash)
            .map(|ids| PostingList::from_vec(ids.clone()))
            .unwrap_or_else(|| PostingList::from_vec(vec![]))
    }

    /// Look up multiple n-grams and intersect (AND semantics).
    pub fn lookup_intersect(&self, hashes: &[NgramHash]) -> PostingList {
        if hashes.is_empty() {
            // No filter: all overlay files are candidates
            let all_ids: Vec<FileId> = (0..self.files.len())
                .map(|i| self.base_file_id + i as FileId)
                .collect();
            return PostingList::from_vec(all_ids);
        }

        let mut iter = hashes.iter();
        let first = *iter.next().unwrap();
        let mut result = self.lookup(first);

        for &hash in iter {
            if result.is_empty() {
                break;
            }
            let posting = self.lookup(hash);
            result = result.intersect(&posting);
        }

        result
    }

    /// Check if a file path has been marked as deleted.
    pub fn is_deleted(&self, rel_path: &Path) -> bool {
        self.deleted.contains(rel_path)
    }

    /// Get the file path for an overlay file ID.
    pub fn file_path(&self, file_id: FileId) -> Option<&Path> {
        if file_id < self.base_file_id {
            return None;
        }
        let index = (file_id - self.base_file_id) as usize;
        self.files.get(index).map(|p| p.as_path())
    }

    /// Get the number of files in the overlay.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Get the number of deleted files.
    pub fn deleted_count(&self) -> usize {
        self.deleted.len()
    }

    /// Check if the overlay is empty (no modified/added/deleted files).
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.deleted.is_empty()
    }

    /// Get all deleted file paths.
    pub fn deleted_files(&self) -> impl Iterator<Item = &Path> {
        self.deleted.iter().map(|p| p.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_empty() {
        let overlay = OverlayIndex::new(1_000_000_000);
        assert!(overlay.is_empty());
        assert_eq!(overlay.file_count(), 0);
        assert_eq!(overlay.deleted_count(), 0);
    }

    #[test]
    fn overlay_add_file() {
        let mut overlay = OverlayIndex::new(1_000_000_000);
        overlay.add_file(PathBuf::from("test.txt"), b"hello world");

        assert_eq!(overlay.file_count(), 1);
        assert!(!overlay.is_empty());

        // Check that n-grams were indexed
        let hash = qndx_core::hash_ngram(b"hel");
        let posting = overlay.lookup(hash);
        assert!(!posting.is_empty());
    }

    #[test]
    fn overlay_mark_deleted() {
        let mut overlay = OverlayIndex::new(1_000_000_000);
        overlay.mark_deleted(PathBuf::from("deleted.txt"));

        assert_eq!(overlay.deleted_count(), 1);
        assert!(overlay.is_deleted(Path::new("deleted.txt")));
        assert!(!overlay.is_deleted(Path::new("other.txt")));
    }

    #[test]
    fn overlay_update_file() {
        let mut overlay = OverlayIndex::new(1_000_000_000);
        let path = PathBuf::from("test.txt");

        overlay.add_file(path.clone(), b"original content");
        assert_eq!(overlay.file_count(), 1);

        // Update the same file
        overlay.add_file(path.clone(), b"modified content");
        assert_eq!(overlay.file_count(), 1); // Still 1 file

        // Check that new n-grams are indexed
        let hash = qndx_core::hash_ngram(b"mod");
        let posting = overlay.lookup(hash);
        assert!(!posting.is_empty());
    }

    #[test]
    fn overlay_file_path_resolution() {
        let mut overlay = OverlayIndex::new(1_000_000_000);
        overlay.add_file(PathBuf::from("file1.txt"), b"content1");
        overlay.add_file(PathBuf::from("file2.txt"), b"content2");

        let id1 = 1_000_000_000;
        let id2 = 1_000_000_001;

        assert_eq!(overlay.file_path(id1), Some(Path::new("file1.txt")));
        assert_eq!(overlay.file_path(id2), Some(Path::new("file2.txt")));
        assert_eq!(overlay.file_path(999), None); // Below base
    }

    #[test]
    fn overlay_lookup_intersect() {
        let mut overlay = OverlayIndex::new(1_000_000_000);
        overlay.add_file(PathBuf::from("file1.txt"), b"hello world");
        overlay.add_file(PathBuf::from("file2.txt"), b"hello rust");

        // Both files contain "hel"
        let hash1 = qndx_core::hash_ngram(b"hel");
        // Only file1 contains "wor"
        let hash2 = qndx_core::hash_ngram(b"wor");

        let result = overlay.lookup_intersect(&[hash1, hash2]);
        let ids = result.to_vec();
        assert_eq!(ids.len(), 1); // Only file1 matches both
        assert_eq!(ids[0], 1_000_000_000); // file1's ID
    }

    #[test]
    fn overlay_from_dirty_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Create some test files
        std::fs::write(root.join("modified.txt"), b"modified content").unwrap();
        std::fs::write(root.join("added.txt"), b"new content").unwrap();

        let dirty_files = vec![
            (PathBuf::from("modified.txt"), FileStatus::Modified),
            (PathBuf::from("added.txt"), FileStatus::Added),
            (PathBuf::from("deleted.txt"), FileStatus::Deleted),
        ];

        let overlay = OverlayIndex::from_dirty_files(root, &dirty_files, 1_000_000_000).unwrap();

        assert_eq!(overlay.file_count(), 2); // modified + added
        assert_eq!(overlay.deleted_count(), 1); // deleted
        assert!(overlay.is_deleted(Path::new("deleted.txt")));
    }
}
