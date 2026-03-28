//! qndx-git: Git integration for the freshness model.
//!
//! Placeholder for M5. Currently provides stub types for benchmarking.

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

/// Stub: detect dirty files in the working tree.
/// Returns file paths and their status.
pub fn detect_dirty_files(_repo_path: &str) -> Vec<(String, FileStatus)> {
    // Stub implementation for M0 benchmarking
    Vec::new()
}

/// Stub: get the HEAD commit hash.
pub fn head_commit(_repo_path: &str) -> Option<String> {
    // Stub implementation for M0 benchmarking
    None
}
