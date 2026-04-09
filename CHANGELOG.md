# Changelog

All notable changes to qndx are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.2.0] - 2026-04-09

### Added
- Incremental index update path: `qndx index` now checks for Git changes since `Manifest.base_commit` and skips rebuild when up to date.
- New `--full` option on `qndx index` to force a full rebuild.
- `qndx-git` APIs for incremental workflows:
  - `GitRepo::detect_changes_since(base_commit)`
  - `GitRepo::commit_exists(rev)`
- Incremental update result reporting in `qndx-index` via `IncrementalResult` and `update_index_from_dir()`.
- Test coverage for nested untracked detection and changes-since-base behavior in `qndx-git`.
- Test coverage for incremental skip/rebuild behavior in `qndx-index`.
- `qndx stats` subcommand: displays index statistics including n-gram distribution (trigram-only, sparse) and posting list distribution (mean, median, P95, P99, max size, and count of large lists).

### Changed
- Hashing backend migrated to rapidhash:
  - file payload integrity checksum now uses rapidhash-v3 (`u64`) in file headers
  - n-gram hashing now uses rapidhash-v3 truncated to `u32` for on-disk compatibility
- Index file format updated to version `2` with a 24-byte header (`magic + version + payload_len + u64 checksum`).
- `qndx-git::detect_dirty_files()` now uses Git porcelain output instead of mtime/size heuristics, improving correctness for modified/added/deleted/untracked detection.
- `qndx index` now persists HEAD commit in the manifest when indexing a Git repository and reuses it for incremental checks.
- Documentation updated for incremental indexing behavior and `--full` usage.

### Documentation
- Updated index format and architecture docs to match current implementation and `qndx-git` APIs.

## [0.1.2] - 2026-04-09

### Changed
- `qndx-cli` now builds the release binary as `qndx` (for example: `./target/release/qndx`).
- Release packaging workflow now copies `qndx`/`qndx.exe` directly from `target/<target>/release/`.

## [0.1.1] - 2026-04-09

### Changed
- Feature-gated CLI benchmark commands behind `bench-tools`; default `qndx-cli` build no longer requires `qndx-bench`.
- Inlined benchmark report and budget-check logic in `qndx-cli` under the `bench-tools` feature.
- Updated benchmark CI invocation to enable `bench-tools` for `qndx bench check-budgets`.
- Added workspace-level package metadata (`repository`, `homepage`) and inherited it across all crates for cleaner publish metadata.

### Fixed
- Resolved `cargo publish --dry-run` failure for `qndx-core` caused by dev-dependencies on unpublished internal crates.
- Moved differential integration tests from `qndx-core/tests` to `qndx-query/tests` to remove publish-order cycles.

## [0.1.0] - 2026-03-30

Initial MVP release. Index-backed regex search with benchmark-driven architecture.

### Added

#### M0: Benchmark foundation
- Criterion benchmark harness with 6 dedicated bench targets
- Deterministic synthetic corpus generation (small/medium/large) with fixed RNG seed
- Benchmark report generation (human-readable and JSON)
- Benchmark targets: `serializer_choice`, `postings_choice`, `ngram_extract`, `query_planner`, `end_to_end_search`, `git_overlay`

#### M1: Scan-only correctness baseline
- File discovery with `.gitignore` and `.ignore` support via `ignore` crate
- Binary file detection (null-byte heuristic on first 8 KB)
- Scan-only regex search with line/column position extraction
- Deterministic sorted output
- Test suite for regex edge cases (41 tests: alternation, character classes, escapes, Unicode, anchors, repetition)

#### M2: Trigram index v0
- Index builder: extract overlapping trigrams, build inverted index, serialize to `ngrams.tbl` + `postings.dat` + `manifest.bin`
- File format with versioned headers (magic bytes, CRC32 checksums)
- Index reader with binary search lookup
- Candidate resolution via posting list intersection (AND) and union (OR)
- Verification pass: run original regex against candidate files
- Differential tests: index results match scan-only results (no false negatives)

#### M3: Sparse n-grams and planner optimization
- Sparse n-gram extraction (variable-length, hash-weighted boundaries)
- Query decomposition into literal segments with alternation support
- Cost-based query planner evaluating trigram vs sparse strategies
- Selectivity estimators: `HashSelectivity` (default), `FrequencySelectivity`
- Sparse covering algorithm (minimal lookup set)

#### M4: Postings optimization and serialization decisions
- Three posting representations: Vec (sorted u32), Roaring bitmap, hybrid (auto-select by cardinality)
- Three on-disk encodings: fixed-width delta (`0x01`), varint delta (`0x02`), Roaring native (`0x03`)
- Tagged format with 1-byte prefix for auto-detection
- Decision Gate A resolved: postcard chosen for manifest serialization
- Decision Gate B resolved: hybrid postings with threshold at 64 entries
- Decision Gate C resolved: planner-selected strategy per query

#### M5: Freshness model (Git overlay)
- Git integration via `gix`: HEAD commit tracking, dirty file detection
- Overlay index: in-memory mini-index for modified/added/untracked files
- Merged query path: baseline index + overlay with deleted file exclusion
- Read-your-writes semantics verified by end-to-end freshness tests

#### M6: Regression tracking
- Performance budgets per benchmark group (`benchmarks/budgets.toml`)
- CI workflow (`.github/workflows/benchmarks.yml`) for automated baseline comparison
- `qndx bench check-budgets` CLI command for budget enforcement
- Regression triage checklist (`docs/regression-triage.md`)
- Release gate criteria and MVP definition (`docs/release-gate.md`)
- Decision gates documentation (`docs/decision-gates.md`)

#### Query diagnostics
- `--strategy` flag on `search` command: `auto` (default), `trigram`, `sparse`
- `plan` subcommand: shows decomposition, both strategies with costs, and selection
- Strategy reporting in `--stats` output

#### Real corpus benchmarks
- `benches/real_corpus.rs`: Criterion benchmark target for real codebases
- Environment variable configuration: `QNDX_BENCH_CORPUS`, `QNDX_BENCH_PATTERNS`, `QNDX_BENCH_NAME`, `QNDX_BENCH_MAX_FILES`
- Benchmark groups: index build, indexed search (auto/trigram/sparse), scan search
- Summary table with candidate counts, match counts, and scan-vs-index speedup
- Example pattern files for Linux kernel and Rust codebases
- Automatic skip when `QNDX_BENCH_CORPUS` is not set

#### Memory-mapped index reader
- `IndexReader` uses `memmap2` for `ngrams.tbl` and `postings.dat`
- Binary search directly over mmap'd bytes (4-byte hash field per comparison)
- Near-instant open time (mmap syscall vs reading full file)
- OS-managed paging: only touched pages consume physical RAM
- CRC32 validation retained at open time

#### Streaming index build
- `build_index_from_dir()` reads files one at a time
- Peak memory reduced from O(corpus) to O(largest_file) + O(inverted_index)
- Build benchmark skipped for corpora exceeding 10K files

#### Documentation
- README.md with quick start, CLI reference, architecture overview, and benchmarking guide
- `docs/architecture.md` -- crate structure, data flow, and design rationale
- `docs/file-format.md` -- on-disk index format specification (v1)
- CHANGELOG.md

### Changed
- CI workflow updated to actions/checkout v6, actions/cache v5, actions/upload-artifact v7, actions/github-script v8
- Switched CI to self-hosted runners
- Benchmark report generation refactored for improved output

### Fixed
- Benchmark regression regex patterns
- Benchmark reports and PR CI workflow
- `rand` crate upgrade to 0.10 compatibility
- `toml` crate upgrade to v1 compatibility
- `roaring` crate upgrade to 0.11 compatibility

### Architecture

- 6 workspace crates: `qndx-core`, `qndx-index`, `qndx-query`, `qndx-git`, `qndx-cli`, `qndx-bench`
- 202 tests (unit, integration, differential, property, edge case)
- Index format v1 with forward-compatible versioned headers
