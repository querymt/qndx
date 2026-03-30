# Architecture

This document describes the crate structure, data flow, and key design decisions in qndx.

## Crate layout

```
crates/
  qndx-core/     Foundation: types, file format, hashing, file walk, scan-only search
  qndx-index/    Index construction and reading (mmap-backed)
  qndx-query/    Pattern analysis, query planning, candidate resolution, verification
  qndx-git/      Git integration (dirty detection, commit tracking)
  qndx-cli/      CLI entrypoints
  qndx-bench/    Benchmark fixtures, reporting, budget enforcement
```

### Dependency graph

```
qndx-cli
  ├── qndx-core
  ├── qndx-index ── qndx-core, qndx-git
  ├── qndx-query ── qndx-core, qndx-index
  ├── qndx-bench ── qndx-core
  └── qndx-git   ── (gix)
```

`qndx-core` has no internal dependencies. Every other crate depends on it for shared types and utilities.

## Crate responsibilities

### qndx-core

The foundation layer. Contains nothing index-specific -- just the primitives that every other crate needs.

- **Types** (`types.rs`): `FileId` (u32), `NgramHash` (u32), `NgramEntry`, `Manifest`
- **Format** (`format.rs`): File header layout (20 bytes: magic, version, payload length, CRC32), read/write/validate functions, n-gram entry serialization, postings encoding (fixed-width delta, varint delta)
- **Hash** (`hash.rs`): CRC32-based n-gram hashing (`hash_ngram`), pair weighting for sparse n-gram selectivity
- **Walk** (`walk.rs`): File discovery using the `ignore` crate. Respects `.gitignore`, `.ignore`, file size limits, binary detection. Returns sorted results for determinism.
- **Scan** (`scan.rs`): Scan-only search (no index). Serves as the correctness oracle -- index-backed results must match scan results. Extracts matches with line/column positions.

### qndx-index

Builds and reads the on-disk index.

- **Builder** (`builder.rs`): Walks files, extracts trigrams and sparse n-grams, builds the inverted index (`BTreeMap<NgramHash, Vec<FileId>>`), serializes to `ngrams.tbl` + `postings.dat` + `manifest.bin`. Two entry points:
  - `build_index()`: from in-memory file data (used by tests and benchmarks)
  - `build_index_from_dir()`: streaming -- reads files one at a time to avoid loading the entire corpus into memory

- **Reader** (`reader.rs`): Memory-mapped index reader. Opens `ngrams.tbl` and `postings.dat` via `memmap2`, validates headers and CRC32, then provides:
  - `lookup(hash)`: binary search over mmap'd n-gram table, returns posting list
  - `lookup_intersect(hashes)`: AND semantics (all n-grams must match)
  - `lookup_union(hashes)`: OR semantics (any n-gram matches)
  - `file_path(id)`: resolve FileId to path via manifest

  Binary search reads only the 4-byte hash field at each step (not the full 20-byte entry), minimizing cache pressure. Postings are sliced directly from the mmap'd region with no copying.

- **N-gram** (`ngram.rs`): Trigram extraction (overlapping 3-byte windows) and sparse n-gram extraction (variable-length n-grams selected by hash-based weight function). Both produce `NgramHash` values.

- **Postings** (`postings.rs`): Three representations:
  - `Vec<u32>`: sorted file IDs, good for small lists
  - `Roaring`: compressed bitmap, efficient for large/dense lists
  - Hybrid: auto-selects based on cardinality threshold (default: 64)

  Tagged on-disk format (1-byte prefix) for transparent encoding:
  - `0x01`: fixed-width delta
  - `0x02`: varint delta (default for Vec)
  - `0x03`: Roaring native serialization

- **Overlay** (`overlay.rs`): In-memory mini-index for dirty files. Built from Git working tree changes. Merged with the baseline index at query time for read-your-writes semantics.

### qndx-query

Analyzes patterns and executes searches.

- **Decompose** (`decompose.rs`): Extracts literal segments from regex patterns. Handles top-level alternation (`a|b` produces OR branches), character classes, escapes. Produces both trigram hashes and sparse n-gram hashes for each literal segment.

- **Planner** (`planner.rs`): Evaluates two strategies per query:
  - **Trigram plan**: use all overlapping trigrams from extracted literals
  - **Sparse plan**: use sparse n-gram covering (fewer, longer grams)

  Picks the lower-cost strategy using a selectivity estimator. Supports `StrategyOverride` (Auto, ForceTrigram, ForceSparse) for testing and diagnostics.

  `plan_diagnostics()` exposes both strategies with costs and lookup counts without committing to a choice.

- **Search** (`search.rs`): Full search pipeline:
  1. Plan the query (choose strategy)
  2. Resolve candidates from the index (intersect/union posting lists)
  3. Read each candidate file from disk
  4. Verify with the full regex
  5. Extract match positions (line, column, text)

  Also provides `index_search_with_overlay()` for the freshness path.

- **Verify** (`verify.rs`): Boolean and position-extracting verification of candidates against the original regex.

### qndx-git

Git integration via [gix](https://crates.io/crates/gix).

- `open_repo()`: Open a Git repository
- `head_commit()`: Get the current HEAD commit hash
- `dirty_files()`: Detect modified, added, deleted, and untracked files in the working tree

Returns `Vec<(PathBuf, FileStatus)>` where `FileStatus` is `Modified`, `Added`, `Deleted`, or `Untracked`.

### qndx-cli

CLI entrypoints using [clap](https://crates.io/crates/clap).

Commands: `index`, `search`, `plan`, `bench report`, `bench check-budgets`.

The search command auto-detects the index (looks for `.qndx/index/v1/ngrams.tbl`) and falls back to scan-only if not present.

### qndx-bench

Benchmark infrastructure.

- **Fixtures** (`fixtures.rs`): Deterministic synthetic corpus generation (seeded RNG). Three sizes: small (50 files), medium (200), large (1000). Also: external corpus loader, pattern file parser, standard corpus discovery from `benchmarks/corpora.toml`, and helpers for real-corpus benchmarks.

- **Report** (`report.rs`): Parses Criterion output, generates human-readable and JSON reports, checks performance budgets from `benchmarks/budgets.toml`.

- **Bench targets**: `serializer_choice`, `postings_choice`, `ngram_extract`, `query_planner`, `end_to_end_search`, `git_overlay`, `real_corpus`.

#### Standard benchmark corpora

Real-world benchmarking uses a tiered set of well-known open-source repositories defined in `benchmarks/corpora.toml`. Each corpus has a tier, associated patterns file, and is downloaded via `benchmarks/fetch_corpora.sh`.

| Tier | Corpus | Files | Size | Language | Use case |
|------|--------|-------|------|----------|----------|
| small | `rust` (rust-lang/rust) | ~35K | ~500 MB | Rust | Fast local iteration |
| medium | `linux` (torvalds/linux) | ~75K | ~1.2 GB | C | Industry-standard grep benchmark |
| medium | `kubernetes` (kubernetes/kubernetes) | ~15K | ~200 MB | Go | Multi-language coverage |
| large | `chromium` (chromium/src) | ~400K | ~20 GB | C++ | Stress testing |

These are the same corpora used by Zoekt, ripgrep, and other code-search tools, making results directly comparable.

**Quick start:**

```bash
# Download standard corpora (excludes large tier by default)
./benchmarks/fetch_corpora.sh

# Run benchmarks against all downloaded corpora
./benchmarks/run_standard_benches.sh

# Run against a single corpus
QNDX_BENCH_CORPUS=benchmarks/corpora/linux \
QNDX_BENCH_PATTERNS=benchmarks/patterns/linux.txt \
cargo bench --bench real_corpus
```

Corpus-specific patterns live in `benchmarks/patterns/<name>.txt` and are merged with the generic benchmark patterns at runtime. See `benchmarks/corpora.toml` for the full configuration.

## Data flow

### Index build

```
discover_files(root, config)
    |
    v
for each file:
    read content
    extract_trigrams(content) ──> inverted[hash].push(file_id)
    extract_sparse_ngrams(content) ──> inverted[hash].push(file_id)
    drop content
    |
    v
for each (hash, file_ids) in inverted:
    PostingList::from_vec_with_threshold(file_ids, 64)
    encode_auto() ──> append to postings_payload
    NgramEntry { hash, offset, len, flags } ──> ngram_entries
    |
    v
write ngrams.tbl  (header + sorted ngram entries)
write postings.dat (header + concatenated posting blocks)
write manifest.bin (header + postcard-serialized Manifest)
```

### Query

```
pattern: "enum AgentMode"
    |
    v
decompose_pattern(pattern)
    literals: ["enum AgentMode"]
    trigram hashes: [12 hashes]
    sparse grams: [13 grams]
    |
    v
plan_query(pattern)
    trigram cost: 12.0 (12 lookups)
    sparse cost: N/A (13 >= 12, no reduction)
    selected: Trigram
    |
    v
resolve_candidates(reader, plan)
    lookup(hash_1) ──> PostingList [0, 5, 12, ...]
    lookup(hash_2) ──> PostingList [5, 12, 99, ...]
    ...
    intersect all ──> candidate_ids [5, 12]
    |
    v
for each candidate_id:
    path = reader.file_path(candidate_id)
    content = fs::read(root.join(path))
    if regex.is_match(content):
        extract match positions
        |
        v
verified matches (sorted by path, line, column)
```

### Freshness (Git overlay)

```
dirty_files(repo)
    ──> [(main.rs, Modified), (new.rs, Added), (old.rs, Deleted)]
    |
    v
OverlayIndex::from_dirty_files(root, dirty_files)
    read modified/added files
    extract trigrams + sparse n-grams
    build mini inverted index
    track deleted file set
    |
    v
index_search_with_overlay(reader, overlay, root, pattern)
    baseline candidates = resolve from reader
    overlay candidates = resolve from overlay
    exclude deleted files from baseline
    merge candidate sets
    verify all candidates
    |
    v
combined results (read-your-writes)
```

## Key design decisions

### Benchmark-driven architecture

Architecture decisions are backed by Criterion benchmarks, not intuition. Three explicit decision gates are defined in [docs/decision-gates.md](decision-gates.md):

- **Gate A (Serializer)**: postcard chosen over wincode for manifest serialization. Simpler, stable wire format, and wincode didn't show sufficient advantage.
- **Gate B (Postings)**: Hybrid (Vec + Roaring) with threshold at 64 entries. Vec with varint delta encoding for small lists, Roaring for large lists.
- **Gate C (N-gram strategy)**: Planner-selected per query. Both trigram and sparse n-grams are stored in the index. The planner picks the lower-cost strategy at query time.

### Memory-mapped reader

The index reader uses `memmap2` for `ngrams.tbl` and `postings.dat`. This means:
- Near-instant open time (no 2 GB allocation for large indexes)
- OS-managed paging (only touched pages consume physical RAM)
- Binary search directly over mmap'd bytes

CRC32 validation is still performed at open time for corruption detection.

### No false negatives

The index is a candidate filter, not the final answer. Every candidate is verified against the original regex by reading the actual file content. This guarantees correctness:

- Index-backed results are a superset of scan-only results
- Differential tests (`tests/differential.rs`) verify this property across multiple corpus sizes and query patterns
- The scan-only path (`qndx-core/src/scan.rs`) serves as the correctness oracle

### Streaming build

`build_index_from_dir()` reads files one at a time rather than loading the entire corpus into memory. For a 1 GB corpus, this reduces peak memory from O(corpus) to O(largest_file) + O(inverted_index).

The inverted index itself (`BTreeMap<NgramHash, Vec<FileId>>`) still requires full in-memory residence. External-sort-based building is future work.

### Deterministic output

All outputs are deterministic:
- File discovery is sorted by path
- Matches are sorted by (path, line, column)
- Benchmark fixtures use a fixed RNG seed
- N-gram table is sorted by hash (BTreeMap iteration order)

This makes differential testing, debugging, and benchmark comparison reliable.
