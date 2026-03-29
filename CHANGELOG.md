# Changelog

All notable changes to qndx are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- README.md with quick start, CLI reference, architecture overview, and benchmarking guide
- `docs/architecture.md` -- crate structure, data flow, and design rationale
- `docs/file-format.md` -- on-disk index format specification (v1)
- CHANGELOG.md

## [0.1.0] - 2026-03-28

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

### Architecture

- 6 workspace crates: `qndx-core`, `qndx-index`, `qndx-query`, `qndx-git`, `qndx-cli`, `qndx-bench`
- 202 tests (unit, integration, differential, property, edge case)
- Index format v1 with forward-compatible versioned headers
