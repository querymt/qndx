//! File discovery and ignore handling.
//!
//! Walks a repository root, respecting `.gitignore` and `.ignore` rules,
//! with configurable file-size limits and binary file detection.

use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

/// Configuration for file discovery.
#[derive(Debug, Clone)]
pub struct WalkConfig {
    /// Maximum file size in bytes. Files larger than this are skipped.
    /// Default: 1 MB.
    pub max_file_size: u64,
    /// Whether to follow symlinks. Default: false.
    pub follow_symlinks: bool,
    /// Whether to respect `.gitignore` files. Default: true.
    pub use_gitignore: bool,
    /// Whether to respect `.ignore` files. Default: true.
    pub use_dot_ignore: bool,
    /// Whether to skip binary files. Default: true.
    pub skip_binary: bool,
    /// Whether to include hidden files/directories. Default: false (skip hidden).
    pub include_hidden: bool,
}

impl Default for WalkConfig {
    fn default() -> Self {
        Self {
            max_file_size: 1_048_576, // 1 MB
            follow_symlinks: false,
            use_gitignore: true,
            use_dot_ignore: true,
            skip_binary: true,
            include_hidden: false,
        }
    }
}

/// A discovered file with its path and content.
#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    /// Path relative to the walk root.
    pub rel_path: String,
    /// Absolute path on disk.
    pub abs_path: PathBuf,
}

/// Walk a directory tree and return all non-ignored, non-binary files.
///
/// Returns files sorted by relative path for deterministic output.
pub fn discover_files(root: &Path, config: &WalkConfig) -> Vec<DiscoveredFile> {
    // Canonicalize root once so strip_prefix works consistently,
    // including on macOS where /tmp -> /private/tmp.
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let walker = WalkBuilder::new(&canonical_root)
        .git_ignore(config.use_gitignore)
        .git_global(config.use_gitignore)
        .git_exclude(config.use_gitignore)
        .require_git(false) // respect .gitignore even outside a git repo
        .ignore(config.use_dot_ignore)
        .hidden(!config.include_hidden)
        .follow_links(config.follow_symlinks)
        .max_filesize(Some(config.max_file_size))
        .build();

    let mut files: Vec<DiscoveredFile> = walker
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            // Only regular files
            entry.file_type().map(|ft| ft.is_file()).unwrap_or(false)
        })
        .filter_map(|entry| {
            let abs_path = entry.path().to_path_buf();

            // Binary detection: check first 8KB for null bytes
            if config.skip_binary {
                if is_likely_binary(&abs_path) {
                    return None;
                }
            }

            let rel_path = abs_path
                .strip_prefix(&canonical_root)
                .unwrap_or(&abs_path)
                .to_string_lossy()
                .to_string();

            Some(DiscoveredFile { rel_path, abs_path })
        })
        .collect();

    // Sort for deterministic output
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    files
}

/// Heuristic binary file detection: read the first 8KB and check for null bytes.
fn is_likely_binary(path: &Path) -> bool {
    use std::fs::File;
    use std::io::Read;

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut buf = [0u8; 8192];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };

    buf[..n].contains(&0)
}

/// Walk and read all discovered files, returning (relative_path, content) pairs.
///
/// Files that cannot be read are silently skipped.
/// Results are sorted by relative path for deterministic output.
pub fn discover_and_read_files(root: &Path, config: &WalkConfig) -> Vec<(String, Vec<u8>)> {
    let files = discover_files(root, config);
    let mut results: Vec<(String, Vec<u8>)> = files
        .into_iter()
        .filter_map(|f| {
            std::fs::read(&f.abs_path)
                .ok()
                .map(|content| (f.rel_path, content))
        })
        .collect();
    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: create a temp directory with test files.
    fn setup_test_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Create some regular files
        fs::write(root.join("hello.rs"), b"fn main() {}").unwrap();
        fs::write(root.join("lib.rs"), b"pub mod hello;").unwrap();

        // Create a subdirectory with files
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/mod.rs"), b"pub fn foo() {}").unwrap();

        // Create a .gitignore
        fs::write(root.join(".gitignore"), "ignored.txt\nbuild/\n").unwrap();

        // Create ignored files
        fs::write(root.join("ignored.txt"), b"should be ignored").unwrap();
        fs::create_dir_all(root.join("build")).unwrap();
        fs::write(root.join("build/output.o"), b"binary stuff").unwrap();

        // Create a binary file (contains null bytes)
        let mut binary_content = vec![0u8; 100];
        binary_content[0] = 0x7f;
        binary_content[1] = b'E';
        binary_content[2] = b'L';
        binary_content[3] = b'F';
        fs::write(root.join("binary.bin"), &binary_content).unwrap();

        // Create a .ignore file
        fs::write(root.join(".ignore"), "also_ignored.log\n").unwrap();
        fs::write(root.join("also_ignored.log"), b"ignored by .ignore").unwrap();

        dir
    }

    #[test]
    fn discovers_regular_files() {
        let dir = setup_test_dir();
        let files = discover_files(dir.path(), &WalkConfig::default());
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();

        assert!(paths.contains(&"hello.rs"));
        assert!(paths.contains(&"lib.rs"));
        assert!(paths.contains(&"src/mod.rs"));
    }

    #[test]
    fn respects_gitignore() {
        let dir = setup_test_dir();
        let files = discover_files(dir.path(), &WalkConfig::default());
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();

        assert!(!paths.contains(&"ignored.txt"));
        assert!(!paths.iter().any(|p| p.starts_with("build/")));
    }

    #[test]
    fn respects_dot_ignore() {
        let dir = setup_test_dir();
        let files = discover_files(dir.path(), &WalkConfig::default());
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();

        assert!(!paths.contains(&"also_ignored.log"));
    }

    #[test]
    fn skips_binary_files() {
        let dir = setup_test_dir();
        let files = discover_files(dir.path(), &WalkConfig::default());
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();

        assert!(!paths.contains(&"binary.bin"));
    }

    #[test]
    fn includes_binary_when_configured() {
        let dir = setup_test_dir();
        let config = WalkConfig {
            skip_binary: false,
            ..Default::default()
        };
        let files = discover_files(dir.path(), &config);
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();

        // binary.bin should be included since it's not in .gitignore
        assert!(paths.contains(&"binary.bin"));
    }

    #[test]
    fn respects_file_size_limit() {
        let dir = setup_test_dir();
        // Create a file that exceeds the limit
        let big_content = vec![b'x'; 2_000_000];
        fs::write(dir.path().join("big.txt"), &big_content).unwrap();

        let config = WalkConfig {
            max_file_size: 1_048_576,
            ..Default::default()
        };
        let files = discover_files(dir.path(), &config);
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();

        assert!(!paths.contains(&"big.txt"));
    }

    #[test]
    fn output_is_sorted() {
        let dir = setup_test_dir();
        let files = discover_files(dir.path(), &WalkConfig::default());
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();

        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn skips_hidden_by_default() {
        let dir = setup_test_dir();
        fs::write(dir.path().join(".hidden_file"), b"hidden").unwrap();

        let files = discover_files(dir.path(), &WalkConfig::default());
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();
        assert!(!paths.contains(&".hidden_file"));
        assert!(!paths.contains(&".gitignore"));
        assert!(!paths.contains(&".ignore"));
    }

    #[test]
    fn includes_hidden_when_configured() {
        let dir = setup_test_dir();
        fs::write(dir.path().join(".hidden_file"), b"hidden").unwrap();

        let config = WalkConfig {
            include_hidden: true,
            ..Default::default()
        };
        let files = discover_files(dir.path(), &config);
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();
        assert!(paths.contains(&".hidden_file"));
    }

    #[test]
    fn discover_and_read_returns_content() {
        let dir = setup_test_dir();
        let results = discover_and_read_files(dir.path(), &WalkConfig::default());

        let hello = results.iter().find(|(p, _)| p == "hello.rs");
        assert!(hello.is_some());
        assert_eq!(hello.unwrap().1, b"fn main() {}");
    }

    #[test]
    fn handles_nested_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/.gitignore"), "*.log\n").unwrap();
        fs::write(root.join("sub/keep.rs"), b"keep me").unwrap();
        fs::write(root.join("sub/debug.log"), b"ignore me").unwrap();
        fs::write(root.join("top.log"), b"this is fine").unwrap();

        let files = discover_files(root, &WalkConfig::default());
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();

        assert!(paths.contains(&"sub/keep.rs"));
        assert!(!paths.contains(&"sub/debug.log"));
        // top.log is NOT ignored because the .gitignore is in sub/
        assert!(paths.contains(&"top.log"));
    }

    #[test]
    fn handles_negation_patterns() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(root.join(".gitignore"), "*.txt\n!important.txt\n").unwrap();
        fs::write(root.join("notes.txt"), b"ignored").unwrap();
        fs::write(root.join("important.txt"), b"kept").unwrap();
        fs::write(root.join("code.rs"), b"kept too").unwrap();

        let files = discover_files(root, &WalkConfig::default());
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();

        assert!(!paths.contains(&"notes.txt"));
        assert!(paths.contains(&"important.txt"));
        assert!(paths.contains(&"code.rs"));
    }
}
