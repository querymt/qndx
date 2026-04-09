//! qndx-git: Git integration for the freshness model.
//!
//! Provides gix-based Git operations to support the freshness model:
//! - Pin index state to a specific commit
//! - Detect modified/untracked/deleted files in working tree
//! - Handle read-your-writes semantics for local edits

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
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
    pub fn detect_dirty_files(&self) -> Result<Vec<(PathBuf, FileStatus)>, GitError> {
        let mut changes = self.git_status_porcelain()?;
        changes.sort_by(|a, b| a.0.cmp(&b.0));
        changes.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        Ok(changes)
    }

    /// Detect changed files since a base commit.
    ///
    /// Includes committed history changes, staged/unstaged worktree edits,
    /// and untracked files.
    pub fn detect_changes_since(
        &self,
        base_commit: &str,
    ) -> Result<Vec<(PathBuf, FileStatus)>, GitError> {
        if !self.commit_exists(base_commit)? {
            return Err(GitError::InvalidReference(format!(
                "base commit not found: {}",
                base_commit
            )));
        }

        let mut changes = self.git_diff_name_status(base_commit)?;
        for path in self.git_ls_untracked()? {
            changes.push((path, FileStatus::Added));
        }

        changes.sort_by(|a, b| a.0.cmp(&b.0));
        changes.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        Ok(changes)
    }

    /// Check if a commit exists in this repository.
    pub fn commit_exists(&self, rev: &str) -> Result<bool, GitError> {
        let output = self.run_git([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{}^{{commit}}", rev),
        ])?;
        Ok(output.status.success())
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

    fn run_git<I, S>(&self, args: I) -> Result<Output, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let workdir = self.repo.workdir().ok_or_else(|| {
            GitError::OperationFailed("repository has no working directory".to_string())
        })?;

        let mut cmd = Command::new("git");
        for arg in args {
            cmd.arg(arg.as_ref());
        }

        cmd.current_dir(workdir).output().map_err(GitError::Io)
    }

    fn git_status_porcelain(&self) -> Result<Vec<(PathBuf, FileStatus)>, GitError> {
        let output = self.run_git(["status", "--porcelain", "--untracked-files=all"])?;
        if !output.status.success() {
            return Err(GitError::OperationFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }

        Ok(parse_status_porcelain(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    fn git_diff_name_status(
        &self,
        base_commit: &str,
    ) -> Result<Vec<(PathBuf, FileStatus)>, GitError> {
        let output = self.run_git(["diff", "--name-status", "--no-renames", base_commit, "--"])?;
        if !output.status.success() {
            return Err(GitError::OperationFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut changes = Vec::new();

        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let status = parts.next().unwrap_or_default();
            let path = parts.next().unwrap_or_default();
            if path.is_empty() {
                continue;
            }

            let mapped = match status.chars().next().unwrap_or('M') {
                'A' => FileStatus::Added,
                'D' => FileStatus::Deleted,
                _ => FileStatus::Modified,
            };
            changes.push((PathBuf::from(path), mapped));
        }

        Ok(changes)
    }

    fn git_ls_untracked(&self) -> Result<Vec<PathBuf>, GitError> {
        let output = self.run_git(["ls-files", "--others", "--exclude-standard"])?;
        if !output.status.success() {
            return Err(GitError::OperationFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut paths = Vec::new();
        for line in stdout.lines() {
            if !line.is_empty() {
                paths.push(PathBuf::from(line));
            }
        }
        Ok(paths)
    }
}

fn parse_status_porcelain(stdout: &str) -> Vec<(PathBuf, FileStatus)> {
    let mut changes = Vec::new();

    for line in stdout.lines() {
        if line.len() < 3 {
            continue;
        }

        let code = &line[..2];
        let path_part = &line[3..];

        if code == "??" {
            changes.push((PathBuf::from(path_part), FileStatus::Added));
            continue;
        }

        if path_part.contains(" -> ") {
            let mut parts = path_part.splitn(2, " -> ");
            if let Some(old_path) = parts.next()
                && !old_path.is_empty()
            {
                changes.push((PathBuf::from(old_path), FileStatus::Deleted));
            }
            if let Some(new_path) = parts.next()
                && !new_path.is_empty()
            {
                changes.push((PathBuf::from(new_path), FileStatus::Added));
            }
            continue;
        }

        let status = if code.contains('D') {
            FileStatus::Deleted
        } else if code.contains('A') {
            FileStatus::Added
        } else {
            FileStatus::Modified
        };
        changes.push((PathBuf::from(path_part), status));
    }

    changes
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

    #[test]
    fn test_detect_untracked_nested_file() {
        let dir = setup_test_repo();
        fs::create_dir_all(dir.path().join("nested/deep")).unwrap();
        fs::write(dir.path().join("nested/deep/new.txt"), "new file").unwrap();

        let dirty = detect_dirty_files(dir.path()).unwrap();
        assert!(dirty.iter().any(|(p, s)| {
            p == &PathBuf::from("nested/deep/new.txt") && matches!(s, FileStatus::Added)
        }));
    }

    #[test]
    fn test_detect_changes_since_commit() {
        let dir = setup_test_repo();
        let repo = GitRepo::open(dir.path()).unwrap();
        let base = repo.head_commit().unwrap();

        fs::write(dir.path().join("file1.txt"), "changed").unwrap();
        fs::write(dir.path().join("file3.txt"), "new").unwrap();
        fs::remove_file(dir.path().join("file2.txt")).unwrap();

        let changes = repo.detect_changes_since(&base).unwrap();

        assert!(changes
            .iter()
            .any(|(p, s)| p == &PathBuf::from("file1.txt") && matches!(s, FileStatus::Modified)));
        assert!(
            changes
                .iter()
                .any(|(p, s)| p == &PathBuf::from("file2.txt") && matches!(s, FileStatus::Deleted))
        );
        assert!(
            changes
                .iter()
                .any(|(p, s)| p == &PathBuf::from("file3.txt") && matches!(s, FileStatus::Added))
        );
    }

    #[test]
    fn test_commit_exists() {
        let dir = setup_test_repo();
        let repo = GitRepo::open(dir.path()).unwrap();
        let base = repo.head_commit().unwrap();

        assert!(repo.commit_exists(&base).unwrap());
        assert!(!repo.commit_exists("deadbeef").unwrap());
    }
}
