//! End-to-end integration tests for M5 freshness model.
//!
//! These tests verify the complete freshness workflow:
//! 1. Build baseline index from a git commit
//! 2. Make local changes (modify/add/delete files)
//! 3. Build overlay from dirty files
//! 4. Verify read-your-writes behavior in search results

use qndx_core::walk::WalkConfig;
use qndx_git::GitRepo;
use qndx_index::{IndexReader, OverlayIndex, build_index};
use qndx_query::index_search_with_overlay;
use std::fs;
use std::process::Command;

/// Helper to create a git repository with an initial commit.
fn setup_git_repo_with_commit() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();

    // Initialize git repo
    Command::new("git")
        .args(["init"])
        .current_dir(path)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(path)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(path)
        .output()
        .unwrap();

    // Create initial files
    fs::write(
        path.join("main.rs"),
        "fn main() {\n    println!(\"hello world\");\n}\n",
    )
    .unwrap();
    fs::write(path.join("lib.rs"), "pub const VERSION: &str = \"1.0\";\n").unwrap();
    fs::create_dir_all(path.join("src")).unwrap();
    fs::write(path.join("src/utils.rs"), "pub fn helper() -> u32 { 42 }\n").unwrap();

    // Add and commit
    Command::new("git")
        .args(["add", "."])
        .current_dir(path)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(path)
        .output()
        .unwrap();

    dir
}

#[test]
fn freshness_e2e_modified_file() {
    let dir = setup_git_repo_with_commit();
    let repo_path = dir.path();

    // Build baseline index from the committed state
    let repo = GitRepo::open(repo_path).unwrap();
    let head_commit = repo.head_commit().unwrap();

    let index_dir = repo_path.join(".qndx/index/v1");
    let files = qndx_core::walk::discover_and_read_files(repo_path, &WalkConfig::default());
    build_index(&files, &index_dir, Some(head_commit.clone())).unwrap();

    // Verify baseline search works
    let reader = IndexReader::open(&index_dir).unwrap();
    let empty_overlay = OverlayIndex::new(1_000_000_000);
    let baseline_result =
        index_search_with_overlay(&reader, &empty_overlay, repo_path, "hello world").unwrap();
    assert!(!baseline_result.results.matches.is_empty());

    // Modify a file
    fs::write(
        repo_path.join("main.rs"),
        "fn main() {\n    println!(\"goodbye world\");\n}\n",
    )
    .unwrap();

    // Build overlay from dirty files
    let dirty_files = repo.detect_dirty_files().unwrap();
    assert!(!dirty_files.is_empty(), "should detect modified file");

    let overlay = OverlayIndex::from_dirty_files(repo_path, &dirty_files, 1_000_000_000).unwrap();
    assert_eq!(
        overlay.file_count(),
        1,
        "should have 1 modified file in overlay"
    );

    // Search for NEW content should find it (read-your-writes)
    let new_result =
        index_search_with_overlay(&reader, &overlay, repo_path, "goodbye world").unwrap();
    assert!(
        !new_result.results.matches.is_empty(),
        "should find new content in modified file"
    );
    assert_eq!(new_result.stats.overlay_files, 1);

    // Search for OLD content should NOT find it in the modified file
    let old_result =
        index_search_with_overlay(&reader, &overlay, repo_path, "hello world").unwrap();
    assert!(
        old_result.results.matches.is_empty(),
        "should not find old content in modified file"
    );
}

#[test]
fn freshness_e2e_added_file() {
    let dir = setup_git_repo_with_commit();
    let repo_path = dir.path();

    // Build baseline index
    let repo = GitRepo::open(repo_path).unwrap();
    let head_commit = repo.head_commit().unwrap();

    let index_dir = repo_path.join(".qndx/index/v1");
    let files = qndx_core::walk::discover_and_read_files(repo_path, &WalkConfig::default());
    build_index(&files, &index_dir, Some(head_commit.clone())).unwrap();
    let reader = IndexReader::open(&index_dir).unwrap();

    // Add a new file (not yet committed)
    fs::write(
        repo_path.join("new_feature.rs"),
        "pub const NEW_FEATURE: bool = true;\n",
    )
    .unwrap();

    // Build overlay
    let dirty_files = repo.detect_dirty_files().unwrap();
    let overlay = OverlayIndex::from_dirty_files(repo_path, &dirty_files, 1_000_000_000).unwrap();
    assert!(
        overlay.file_count() > 0,
        "should have added file in overlay"
    );

    // Search for content in new file should find it
    let result = index_search_with_overlay(&reader, &overlay, repo_path, "NEW_FEATURE").unwrap();
    assert!(
        !result.results.matches.is_empty(),
        "should find content in new file"
    );

    let paths: Vec<&str> = result
        .results
        .matches
        .iter()
        .map(|m| m.path.as_str())
        .collect();
    assert!(
        paths.iter().any(|p| p.contains("new_feature.rs")),
        "should find in new_feature.rs"
    );
}

#[test]
fn freshness_e2e_deleted_file() {
    let dir = setup_git_repo_with_commit();
    let repo_path = dir.path();

    // Build baseline index
    let repo = GitRepo::open(repo_path).unwrap();
    let head_commit = repo.head_commit().unwrap();

    let index_dir = repo_path.join(".qndx/index/v1");
    let files = qndx_core::walk::discover_and_read_files(repo_path, &WalkConfig::default());
    build_index(&files, &index_dir, Some(head_commit.clone())).unwrap();
    let reader = IndexReader::open(&index_dir).unwrap();

    // Verify file is searchable before deletion
    let empty_overlay = OverlayIndex::new(1_000_000_000);
    let before_result =
        index_search_with_overlay(&reader, &empty_overlay, repo_path, "VERSION").unwrap();
    assert!(
        !before_result.results.matches.is_empty(),
        "should find VERSION in lib.rs"
    );

    // Delete a file
    fs::remove_file(repo_path.join("lib.rs")).unwrap();

    // Build overlay
    let dirty_files = repo.detect_dirty_files().unwrap();
    let overlay = OverlayIndex::from_dirty_files(repo_path, &dirty_files, 1_000_000_000).unwrap();
    assert!(
        overlay.deleted_count() > 0,
        "should have deleted file in overlay"
    );

    // Search for content in deleted file should NOT find it
    let after_result = index_search_with_overlay(&reader, &overlay, repo_path, "VERSION").unwrap();
    assert!(
        after_result.results.matches.is_empty(),
        "should not find content in deleted file"
    );
    assert_eq!(after_result.stats.deleted_files, overlay.deleted_count());
}

#[test]
fn freshness_e2e_mixed_changes() {
    let dir = setup_git_repo_with_commit();
    let repo_path = dir.path();

    // Build baseline index
    let repo = GitRepo::open(repo_path).unwrap();
    let head_commit = repo.head_commit().unwrap();

    let index_dir = repo_path.join(".qndx/index/v1");
    let files = qndx_core::walk::discover_and_read_files(repo_path, &WalkConfig::default());
    build_index(&files, &index_dir, Some(head_commit.clone())).unwrap();
    let reader = IndexReader::open(&index_dir).unwrap();

    // Make mixed changes:
    // 1. Modify main.rs
    fs::write(
        repo_path.join("main.rs"),
        "fn main() {\n    println!(\"MODIFIED_MAIN\");\n}\n",
    )
    .unwrap();

    // 2. Add new file
    fs::write(
        repo_path.join("added.rs"),
        "pub const ADDED_CONTENT: usize = 123;\n",
    )
    .unwrap();

    // 3. Delete lib.rs
    fs::remove_file(repo_path.join("lib.rs")).unwrap();

    // Build overlay
    let dirty_files = repo.detect_dirty_files().unwrap();
    assert!(dirty_files.len() >= 3, "should detect at least 3 changes");

    let overlay = OverlayIndex::from_dirty_files(repo_path, &dirty_files, 1_000_000_000).unwrap();

    // Test 1: Modified content is searchable
    let mod_result =
        index_search_with_overlay(&reader, &overlay, repo_path, "MODIFIED_MAIN").unwrap();
    assert!(
        !mod_result.results.matches.is_empty(),
        "should find modified content"
    );

    // Test 2: Added content is searchable
    let add_result =
        index_search_with_overlay(&reader, &overlay, repo_path, "ADDED_CONTENT").unwrap();
    assert!(
        !add_result.results.matches.is_empty(),
        "should find added content"
    );

    // Test 3: Deleted file content is not searchable
    let del_result = index_search_with_overlay(&reader, &overlay, repo_path, "VERSION").unwrap();
    assert!(
        del_result.results.matches.is_empty(),
        "should not find content in deleted file"
    );

    // Test 4: Unchanged file (src/utils.rs) is still searchable
    let unchanged_result =
        index_search_with_overlay(&reader, &overlay, repo_path, "helper").unwrap();
    assert!(
        !unchanged_result.results.matches.is_empty(),
        "should find content in unchanged file"
    );
}

#[test]
fn freshness_update_to_query_latency() {
    let dir = setup_git_repo_with_commit();
    let repo_path = dir.path();

    // Build baseline index
    let repo = GitRepo::open(repo_path).unwrap();
    let head_commit = repo.head_commit().unwrap();

    let index_dir = repo_path.join(".qndx/index/v1");
    let files = qndx_core::walk::discover_and_read_files(repo_path, &WalkConfig::default());
    build_index(&files, &index_dir, Some(head_commit.clone())).unwrap();
    let reader = IndexReader::open(&index_dir).unwrap();

    // Make a local edit
    fs::write(
        repo_path.join("main.rs"),
        "fn main() {\n    println!(\"FRESH_EDIT\");\n}\n",
    )
    .unwrap();

    // Measure update-to-query latency
    let start = std::time::Instant::now();

    // Detect dirty files
    let dirty_files = repo.detect_dirty_files().unwrap();

    // Build overlay
    let overlay = OverlayIndex::from_dirty_files(repo_path, &dirty_files, 1_000_000_000).unwrap();

    // Run query
    let result = index_search_with_overlay(&reader, &overlay, repo_path, "FRESH_EDIT").unwrap();

    let elapsed = start.elapsed();

    // Verify correctness
    assert!(!result.results.matches.is_empty(), "should find fresh edit");

    // Check latency target (should be sub-second for small repos)
    // This is a soft check - actual performance will vary
    println!("Update-to-query latency: {:?}", elapsed);
    assert!(
        elapsed.as_secs() < 5,
        "update-to-query should be reasonably fast (< 5s for test)"
    );
}
