//! qndx-git: Git integration for the freshness model.
//!
//! Provides gix-based Git operations to support the freshness model:
//! - Pin index state to a specific commit
//! - Detect modified/untracked/deleted files in working tree
//! - Handle read-your-writes semantics for local edits

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Status of a file relative to the indexed commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// File is unchanged from the indexed commit.
    Clean,
    /// File has been modified in the working tree.
    Modified,
    /// File is new (untracked or added).
    Added,
    /// File has been deleted.
    Deleted,
}

/// Git integration errors.
#[derive(Debug, Error)]
pub enum GitError {
    #[error("not a git repository: {0}")]
    NotARepository(String),
    #[error("git operation failed: {0}")]
    OperationFailed(String),
    #[error("invalid reference: {0}")]
    InvalidReference(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Git repository adapter using gix.
pub struct GitRepo {
    repo: gix::Repository,
}

impl GitRepo {
    /// Open a Git repository at the given path.
    pub fn open(path: &Path) -> Result<Self, GitError> {
        let repo = gix::open(path)
            .map_err(|e| GitError::NotARepository(format!("{}: {}", path.display(), e)))?;
        Ok(Self { repo })
    }

    /// Get the current HEAD commit hash (full SHA as hex string).
    pub fn head_commit(&self) -> Result<String, GitError> {
        let mut head = self
            .repo
            .head()
            .map_err(|e| GitError::OperationFailed(format!("failed to get HEAD: {}", e)))?;

        let commit = head.peel_to_commit().map_err(|e| {
            GitError::InvalidReference(format!("HEAD is not a valid commit: {}", e))
        })?;

        Ok(commit.id.to_hex().to_string())
    }

    /// Detect dirty (modified/added/deleted) files in the working tree.
    /// Returns relative paths from the repository root and their status.
    ///
    /// This compares the working tree against the index (staging area).
    /// Files are considered dirty if they differ from what's in the index.
    pub fn detect_dirty_files(&self) -> Result<Vec<(PathBuf, FileStatus)>, GitError> {
        let mut dirty_files = Vec::new();

        // Get the index (staging area)
        let index = self
            .repo
            .index()
            .map_err(|e| GitError::OperationFailed(format!("failed to read index: {}", e)))?;

        // Build a set of tracked files from the index
        let mut tracked_files: HashSet<PathBuf> = HashSet::new();
        for entry in index.entries() {
            let path_bytes = entry.path(&index);
            let path = PathBuf::from(String::from_utf8_lossy(path_bytes.as_ref()).to_string());
            tracked_files.insert(path);
        }

        // Use gix status to detect changes
        let workdir = self.repo.workdir().ok_or_else(|| {
            GitError::OperationFailed("repository has no working directory".to_string())
        })?;

        // Check for modified and deleted files by comparing index with working tree
        for entry in index.entries() {
            let path_bytes = entry.path(&index);
            let rel_path = PathBuf::from(String::from_utf8_lossy(path_bytes.as_ref()).to_string());
            let abs_path = workdir.join(&rel_path);

            if !abs_path.exists() {
                // File is in index but not in working tree: deleted
                dirty_files.push((rel_path.clone(), FileStatus::Deleted));
            } else {
                // File exists: check if modified
                if let Ok(metadata) = abs_path.metadata() {
                    // Simple check: compare mtime and size
                    // Note: This is a heuristic. A proper implementation would hash content.
                    let entry_mtime_secs = entry.stat.mtime.secs as u64;
                    let file_mtime_secs = metadata
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);

                    let entry_size = entry.stat.size;
                    let file_size = metadata.len();

                    // If mtime or size differs, consider it modified
                    // (This is a simplification; real git compares content hashes)
                    if file_mtime_secs != entry_mtime_secs || file_size != entry_size as u64 {
                        // For more accuracy, we could hash the file content and compare with entry.id
                        dirty_files.push((rel_path.clone(), FileStatus::Modified));
                    }
                }
            }
        }

        // Check for untracked files (files in working tree but not in index)
        // Walk the working directory and find files not in tracked_files set
        if let Ok(entries) = std::fs::read_dir(workdir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    let abs_path = entry.path();
                    if let Ok(rel_path) = abs_path.strip_prefix(workdir) {
                        let rel_path_buf = rel_path.to_path_buf();

                        // Skip .git directory
                        if rel_path_buf.starts_with(".git") {
                            continue;
                        }

                        if file_type.is_file() && !tracked_files.contains(&rel_path_buf) {
                            dirty_files.push((rel_path_buf, FileStatus::Added));
                        }
                    }
                }
            }
        }

        Ok(dirty_files)
    }

    /// Get the commit hash for a specific reference (e.g., "HEAD", "main", a SHA).
    pub fn resolve_reference(&self, refname: &str) -> Result<String, GitError> {
        let mut reference = self.repo.find_reference(refname).map_err(|e| {
            GitError::InvalidReference(format!("failed to find reference '{}': {}", refname, e))
        })?;

        let commit = reference.peel_to_id().map_err(|e| {
            GitError::InvalidReference(format!(
                "reference '{}' is not a valid commit: {}",
                refname, e
            ))
        })?;

        Ok(commit.to_hex().to_string())
    }

    /// Check if the working tree is clean (no modifications).
    pub fn is_clean(&self) -> Result<bool, GitError> {
        let dirty = self.detect_dirty_files()?;
        Ok(dirty.is_empty())
    }

    /// Get the repository root path.
    pub fn root_path(&self) -> Option<&Path> {
        self.repo.workdir()
    }
}

/// Convenience function: detect dirty files in a repository at the given path.
/// Returns file paths (relative to repo root) and their status.
pub fn detect_dirty_files(repo_path: &Path) -> Result<Vec<(PathBuf, FileStatus)>, GitError> {
    let repo = GitRepo::open(repo_path)?;
    repo.detect_dirty_files()
}

/// Convenience function: get the HEAD commit hash.
pub fn head_commit(repo_path: &Path) -> Result<String, GitError> {
    let repo = GitRepo::open(repo_path)?;
    repo.head_commit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper to create a test git repository with some files.
    fn setup_test_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();

        // Initialize git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();

        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .unwrap();

        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .unwrap();

        // Create initial files
        fs::write(path.join("file1.txt"), "content1").unwrap();
        fs::write(path.join("file2.txt"), "content2").unwrap();

        // Add and commit
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(path)
            .output()
            .unwrap();

        std::process::Command::new("git")
            .args(["commit", "-m", "initial commit"])
            .current_dir(path)
            .output()
            .unwrap();

        dir
    }

    #[test]
    fn test_open_repo() {
        let dir = setup_test_repo();
        let repo = GitRepo::open(dir.path());
        assert!(repo.is_ok());
    }

    #[test]
    fn test_head_commit() {
        let dir = setup_test_repo();
        let commit = head_commit(dir.path());
        assert!(commit.is_ok());
        let sha = commit.unwrap();
        assert_eq!(sha.len(), 40); // SHA-1 is 40 hex chars
    }

    #[test]
    fn test_clean_repo() {
        let dir = setup_test_repo();
        let repo = GitRepo::open(dir.path()).unwrap();
        let is_clean = repo.is_clean().unwrap();
        assert!(is_clean);
    }

    #[test]
    fn test_detect_modified_file() {
        let dir = setup_test_repo();

        // Modify a file
        fs::write(dir.path().join("file1.txt"), "modified content").unwrap();

        let dirty = detect_dirty_files(dir.path()).unwrap();
        assert!(!dirty.is_empty());

        let modified_files: Vec<_> = dirty
            .iter()
            .filter(|(_, status)| matches!(status, FileStatus::Modified))
            .collect();
        assert!(!modified_files.is_empty());
    }

    #[test]
    fn test_detect_added_file() {
        let dir = setup_test_repo();

        // Add a new untracked file
        fs::write(dir.path().join("file3.txt"), "new file").unwrap();

        let dirty = detect_dirty_files(dir.path()).unwrap();
        assert!(!dirty.is_empty());

        let added_files: Vec<_> = dirty
            .iter()
            .filter(|(_, status)| matches!(status, FileStatus::Added))
            .collect();
        assert!(!added_files.is_empty());
    }

    #[test]
    fn test_detect_deleted_file() {
        let dir = setup_test_repo();

        // Delete a tracked file
        fs::remove_file(dir.path().join("file1.txt")).unwrap();

        let dirty = detect_dirty_files(dir.path()).unwrap();
        assert!(!dirty.is_empty());

        let deleted_files: Vec<_> = dirty
            .iter()
            .filter(|(_, status)| matches!(status, FileStatus::Deleted))
            .collect();
        assert!(!deleted_files.is_empty());
    }
}
