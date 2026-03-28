# Release Gate and MVP Definition of Done

This document defines the criteria that must be met before a release can be shipped, and specifically what constitutes a complete MVP for qndx.

---

## Release Gate Criteria

Every release must pass **all** of the following gates before it can be published. This applies to MVP and all subsequent releases.

### Gate 1: Correctness ✓

**Requirement**: No functional bugs, no false negatives in search results.

#### Checks:

- [ ] All unit tests pass: `cargo test --all-features`
- [ ] All integration tests pass: `cargo test --all-features --tests`
- [ ] Property tests pass (if applicable): `cargo test --all-features property`
- [ ] Differential tests pass:
  - Index-backed search results match scan-only results (no false negatives)
  - `tests/differential.rs` verifies result equivalence
- [ ] Regression edge cases pass: `tests/regex_edge_cases.rs`
- [ ] Manual smoke tests on reference corpora (documented in test plan)

#### Acceptance:

```bash
# All of these must pass
cargo test --all-features
cargo test --all-features --tests
cargo test --all-features --doc

# Zero test failures allowed
```

---

### Gate 2: Performance Budgets ⚡

**Requirement**: All critical performance budgets are met or explicitly waived with justification.

#### Checks:

- [ ] Run full benchmark suite: `cargo bench`
- [ ] Compare against baseline: `cargo bench -- --baseline main`
- [ ] Check budgets: `cargo run -p qndx-cli -- bench check-budgets`
- [ ] All **critical** budgets pass (from `benchmarks/budgets.toml`):
  - [ ] End-to-end search (literal): p50 ≤ 10% regression
  - [ ] End-to-end search (regex): p50 ≤ 10% regression
  - [ ] Postings intersection: ≤ 15% regression
  - [ ] Query planner: p50 ≤ 10% regression
  - [ ] Index size: ≤ 25% growth
  - [ ] Git overlay operations: ≤ 15% regression

#### Waivers:

If a critical budget is violated but the release should proceed:

1. Document the violation in `benchmarks/results/regressions.md`
2. Provide clear justification (e.g., architecture improvement, feature enablement)
3. Show end-to-end impact is acceptable
4. Get approval from maintainer
5. Create follow-up issue to recover performance (if applicable)

#### Acceptance:

```bash
cargo run -p qndx-cli -- bench check-budgets --fail-on-critical
# Exit code 0 = passed
# Exit code 1 = violations found (requires waiver or fix)
```

---

### Gate 3: Documentation 📚

**Requirement**: Core functionality is documented and examples work.

#### Checks:

- [ ] README.md is up-to-date with current features
- [ ] API docs build without warnings: `cargo doc --all-features --no-deps`
- [ ] Doc tests pass: `cargo test --all-features --doc`
- [ ] Usage examples are present in:
  - [ ] `README.md` (basic usage)
  - [ ] `crates/qndx-cli/README.md` or inline help (CLI usage)
  - [ ] Doc comments on public APIs
- [ ] Migration guide exists (if breaking changes since last release)
- [ ] CHANGELOG.md is updated with release notes

#### Acceptance:

```bash
cargo doc --all-features --no-deps
# No warnings or errors

cargo test --doc
# All doc tests pass
```

---

### Gate 4: Code Quality 🧹

**Requirement**: Code meets quality standards (lints, formatting, basic hygiene).

#### Checks:

- [ ] Clippy passes with no warnings: `cargo clippy --all-features --all-targets -- -D warnings`
- [ ] Formatting is consistent: `cargo fmt --all -- --check`
- [ ] No TODO/FIXME in release-critical paths (grep for markers)
- [ ] No `panic!()` or `unwrap()` in hot paths (manual review of changed files)
- [ ] No security vulnerabilities: `cargo audit` (or equivalent)

#### Acceptance:

```bash
cargo clippy --all-features --all-targets -- -D warnings
cargo fmt --all -- --check
cargo audit
# All pass
```

---

### Gate 5: Build and CI ✅

**Requirement**: Project builds cleanly and CI passes on all supported platforms.

#### Checks:

- [ ] Local build succeeds: `cargo build --release`
- [ ] All CI workflows pass:
  - [ ] Tests workflow
  - [ ] Benchmarks workflow
  - [ ] Lints workflow
  - [ ] (Optional) Platform matrix (Linux, macOS, Windows)
- [ ] No uncommitted changes in release commit
- [ ] Version numbers are updated consistently in:
  - [ ] `Cargo.toml` (workspace)
  - [ ] Individual crate `Cargo.toml` files
  - [ ] `CHANGELOG.md`

#### Acceptance:

```bash
cargo build --release
cargo test --release --all-features
# Both succeed, no errors
```

---

### Gate 6: Manual Validation 🔍

**Requirement**: Key user workflows work as expected in realistic scenarios.

#### Checks:

- [ ] **Index a representative corpus**:
  ```bash
  ./target/release/qndx index --root <large-repo>
  # Completes successfully, reasonable time
  ```

- [ ] **Run common queries**:
  ```bash
  ./target/release/qndx search "TODO" --stats
  ./target/release/qndx search "error.*log" --stats
  ./target/release/qndx search "(foo|bar)" --stats
  # All return correct results, reasonable latency
  ```

- [ ] **Test freshness** (if Git overlay is implemented):
  ```bash
  echo "NEW CONTENT TODO" >> test.txt
  ./target/release/qndx search "NEW CONTENT" --stats
  # Returns the new content (read-your-writes)
  ```

- [ ] **Verify index size**:
  ```bash
  du -sh <repo-dir>
  du -sh <repo-dir>/.qndx/index/v1
  # Index size is reasonable (< 50% of corpus)
  ```

#### Acceptance:

Manual validation checklist completed, no blocking issues found.

---

## MVP Definition of Done

The MVP is considered **done** when all of the following are true:

### 1. Correctness Equivalence ✓

**Requirement**: Index-backed search is functionally equivalent to scan-only search.

#### Criteria:

- [ ] **No false negatives**: Every match found by scan is also found by index
- [ ] **No false positives**: Every match returned by index is verified against file content
- [ ] **Differential tests pass**:
  ```rust
  // From tests/differential.rs
  assert_eq!(
      scan_results.matches.len(),
      index_results.matches.len(),
      "match count mismatch"
  );
  ```

- [ ] **Correctness is verified** on:
  - [ ] Literal queries: `"error"`, `"TODO"`
  - [ ] Simple regex: `"err.*log"`, `"\bfoo\b"`
  - [ ] Complex regex: `"(foo|bar).*baz"`, `"[A-Z][a-z]+"`
  - [ ] Edge cases: Unicode, special chars, escaping

#### Validation:

```bash
cargo test differential
cargo test regex_edge_cases
# All pass
```

---

### 2. Query Latency Wins ⚡

**Requirement**: Index-backed search is faster than scan for target corpora and query classes.

#### Criteria:

- [ ] **Latency improvement** on medium+ corpus (100MB, ~1000 files):
  - Literal queries: ≥ 2x faster than scan
  - Regex queries: ≥ 1.5x faster than scan
  - Complex queries: ≥ 1.3x faster than scan

- [ ] **End-to-end benchmarks** demonstrate wins:
  ```bash
  cargo bench -- end_to_end_search
  # Compare "scan" vs "index" groups
  # Index should be consistently faster on target query classes
  ```

- [ ] **Real-world validation**:
  - Index a 100MB+ repository
  - Run 10-20 representative queries
  - Measure average latency: index < scan

#### Validation:

```bash
# Run end-to-end benchmarks
cargo bench -- end_to_end_search

# Manual timing on real corpus
time ./target/release/qndx search "pattern" --scan
time ./target/release/qndx search "pattern"
# Index version should be measurably faster
```

---

### 3. Git Overlay Freshness Works 🔄

**Requirement**: Local edits are immediately searchable (read-your-writes behavior).

#### Criteria:

- [ ] **Dirty file detection**:
  - Modified files are detected via Git working tree status
  - Untracked files are included in overlay

- [ ] **Overlay updates**:
  - Dirty files are re-indexed on demand or incrementally
  - Overlay index is merged with baseline index at query time

- [ ] **Read-your-writes**:
  ```bash
  # Edit a file
  echo "FRESH CONTENT" >> file.txt
  
  # Search immediately
  ./target/release/qndx search "FRESH CONTENT"
  # Returns the new content (no manual re-index needed)
  ```

- [ ] **Overlay latency** is acceptable:
  - Dirty detection: < 50ms for typical working tree
  - Overlay merge: < 10% of total query time

#### Validation:

```bash
cargo test freshness_e2e
# Test from tests/freshness_e2e.rs (if implemented)

# Manual validation:
# 1. Index a repo
# 2. Edit a file
# 3. Search for content in edited file
# 4. Verify it's found without manual re-index
```

---

### 4. Performance Baselines and Regression Checks Are Automated 🤖

**Requirement**: Regression detection runs automatically on every PR.

#### Criteria:

- [ ] **CI workflow exists**: `.github/workflows/benchmarks.yml`

- [ ] **Baseline management**:
  - Main branch benchmarks saved as baseline
  - PR benchmarks compared against main baseline
  - Baselines stored in CI cache or artifacts

- [ ] **Budget enforcement**:
  - `benchmarks/budgets.toml` defines thresholds
  - `qndx-cli bench check-budgets` validates compliance
  - CI fails on critical budget violations

- [ ] **Trend tracking**:
  - Benchmark results stored as artifacts: `target/criterion/`
  - Historical results preserved for 90+ days
  - (Optional) Results tracked in `benchmarks/results/`

#### Validation:

```bash
# Simulate CI workflow locally
cargo bench -- --save-baseline main
# Make a change that regresses performance
cargo bench -- --baseline main
# Should show regression

cargo run -p qndx-cli -- bench check-budgets
# Should fail if regression exceeds budget
```

---

### 5. Format Versioning and Migration Strategy Are Documented 📖

**Requirement**: Index format is versioned and migration path is clear.

#### Criteria:

- [ ] **File format versioning**:
  - Index files include magic bytes and version number
  - Example: `ngrams.tbl` starts with `b"QNDX"` + version u32
  - Version is checked on index open

- [ ] **Version compatibility**:
  - Document supported version range
  - Older versions are rejected with clear error message
  - (MVP: single version, no backward compatibility required)

- [ ] **Migration strategy documented**:
  - What happens when format changes?
  - How to upgrade an index? (MVP: rebuild from scratch)
  - Future: incremental migration or multi-version support

- [ ] **Documentation exists**:
  - File format spec in `docs/file-format.md` or code comments
  - Migration guide in `docs/migration.md` or README

#### Validation:

```bash
# Check that version is embedded
hexdump -C .qndx/index/v1/ngrams.tbl | head -n 1
# Should show magic bytes + version

# Try to open an incompatible index
# Should fail with clear error, not panic
```

---

## MVP Checklist Summary

Use this checklist to track MVP completion:

```markdown
## MVP Completion Checklist

### Core Requirements

- [ ] 1. Correctness equivalence verified (no false negatives)
- [ ] 2. Query latency wins demonstrated on target corpora
- [ ] 3. Git overlay freshness works (read-your-writes)
- [ ] 4. Performance regression checks automated in CI
- [ ] 5. Format versioning and migration documented

### Release Gates

- [ ] Gate 1: Correctness (all tests pass)
- [ ] Gate 2: Performance budgets (critical budgets met)
- [ ] Gate 3: Documentation (README, API docs, examples)
- [ ] Gate 4: Code quality (clippy, fmt, audit)
- [ ] Gate 5: Build and CI (all workflows pass)
- [ ] Gate 6: Manual validation (key workflows tested)

### Documentation

- [ ] README.md is complete
- [ ] CHANGELOG.md has release notes
- [ ] API docs build without warnings
- [ ] File format is documented
- [ ] Migration strategy is documented

### Artifacts

- [ ] Release binaries built: `cargo build --release`
- [ ] Release tag created: `git tag v0.1.0`
- [ ] GitHub release created with notes
- [ ] (Optional) Crates published: `cargo publish`

---

**MVP is DONE when all checkboxes above are complete.**
```

---

## Post-MVP: Continuous Release Process

After MVP, follow this process for each release:

### 1. Pre-Release

- [ ] Create release branch: `git checkout -b release/v0.x.0`
- [ ] Update version numbers
- [ ] Update CHANGELOG.md
- [ ] Run full test suite
- [ ] Run full benchmark suite
- [ ] Check budgets

### 2. Release Gate Validation

- [ ] Go through all 6 gates (listed above)
- [ ] Triage any violations
- [ ] Get approvals

### 3. Release

- [ ] Merge release branch to main
- [ ] Tag release: `git tag v0.x.0`
- [ ] Build release artifacts: `cargo build --release`
- [ ] Create GitHub release with notes
- [ ] Publish crates (if applicable): `cargo publish`

### 4. Post-Release

- [ ] Monitor for issues
- [ ] Update baseline: `cargo bench -- --save-baseline v0.x.0`
- [ ] Archive benchmark results
- [ ] Announce release

---

## Glossary

**Release Gate**: A mandatory checkpoint that must be passed before shipping a release.

**MVP**: Minimum Viable Product - the smallest release that delivers core value.

**Critical Budget**: A performance threshold that will fail CI if violated.

**Differential Test**: A test that compares index-backed results against scan-only results to ensure correctness.

**Baseline**: A saved benchmark result used for comparison in CI.

---

## Revision History

- **2026-03-28**: Initial version - MVP definition and release gates
