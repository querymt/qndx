# qndx

Fast regex search indexer for large repositories.

qndx builds a local n-gram index over source files and uses it to narrow the search space before running the actual regex. For selective queries on large codebases, this is significantly faster than scanning every file -- while guaranteeing no false negatives.

## How it works

1. **Index**: Extract overlapping trigrams (and sparse n-grams) from every file. Store them in a sorted lookup table (`ngrams.tbl`) with postings lists (`postings.dat`) that map each n-gram to the files containing it.

2. **Search**: Decompose the regex into required literal fragments, look up their n-gram hashes in the index, intersect the posting lists to get a small candidate set, then run the full regex only against those candidates.

3. **Freshness**: Track Git working tree state. Modified, added, and untracked files are re-indexed into a lightweight overlay that merges with the baseline index at query time, giving read-your-writes semantics without a full rebuild.

Every match returned by the index path is verified against the actual file content. The index only eliminates files that provably cannot match -- it never introduces false negatives.

## Quick start

```bash
# Build
cargo build --release

# Index a repository
qndx index -r /path/to/repo

# Search
qndx search -r /path/to/repo "fn main"
qndx search -r /path/to/repo "TODO|FIXME|HACK" --stats
qndx search -r /path/to/repo "impl.*Iterator" --strategy trigram --stats

# Inspect the query plan without searching
qndx plan "DatabaseConnection"
```

The index is stored in `<root>/.qndx/index/v1/` and reused automatically on subsequent searches. If no index exists, `search` falls back to a full scan.

## Performance

Measured on a 722-file / 8.6 MB Rust codebase (querymt):

| Query | Strategy | Candidates | Scan | Indexed | Speedup |
|-------|----------|-----------|------|---------|---------|
| `enum AgentMode` | trigram | 8 / 722 | 27 ms | 0.008 ms | 3375x |
| `TODO` | trigram | 45 / 722 | 27 ms | 1.4 ms | 19x |
| `self\.\w+` | trigram | 214 / 722 | 28 ms | 6.9 ms | 4x |
| `pub fn` | trigram | 240 / 722 | 27 ms | 6.7 ms | 4x |
| `impl.*for` | trigram | 427 / 722 | 28 ms | 9.9 ms | 3x |

The index reader uses memory-mapped I/O (`memmap2`), so query-time resident memory is proportional to pages touched during the search, not the index size. A 2 GB index (Linux kernel) requires only ~100 KB of resident memory for a typical query.

## CLI reference

### `qndx index`

Build the search index for a directory.

```
qndx index [OPTIONS]

Options:
  -r, --root <ROOT>                    Root directory to index [default: .]
  -i, --index-dir <INDEX_DIR>          Index output directory
      --max-file-size <MAX_FILE_SIZE>  Maximum file size in bytes [default: 1048576]
      --hidden                         Include hidden files
      --binary                         Include binary files
```

### `qndx search`

Search using regex, with optional index acceleration.

```
qndx search [OPTIONS] <PATTERN>

Options:
  -r, --root <ROOT>            Root directory to search [default: .]
  -i, --index-dir <INDEX_DIR>  Index directory
  -l, --files-only             Show only file names
      --stats                  Show timing and candidate statistics
      --scan                   Force scan-only mode (ignore index)
      --strategy <STRATEGY>    N-gram strategy: auto, trigram, sparse [default: auto]
```

Output format: `path:line:column: matched_text`

When `--stats` is enabled for indexed search, qndx collects and prints a summary plus stage timings:

```
3 matches in 8 files (174185 bytes, 8 candidates / 722 total, 12 lookups, strategy: trigram) in 0.008s [indexed]
  timing: open=3.412ms, plan=0.071ms, candidates=0.204ms, verify=4.033ms
```

### `qndx plan`

Show the query plan for a pattern without running a search.

```
qndx plan [OPTIONS] <PATTERN>

Options:
      --strategy <STRATEGY>  Force a specific strategy [default: auto]
```

Example output:

```
Pattern: enum AgentMode

Literals: ["enum AgentMode"]

Trigram plan:
  lookups: 12
  cost:    12.00

Sparse plan: unavailable (13 sparse grams >= 12 trigrams, no reduction)

Selected:  trigram
Lookups:   12
Cost:      12.00
```

### `qndx bench` (feature-gated)

Benchmark reporting and budget checking (see [Benchmarking](#benchmarking)). This command is available only when `qndx-cli` is built with the `bench-tools` feature.

```bash
cargo run -p qndx-cli --features bench-tools -- bench report
cargo run -p qndx-cli --features bench-tools -- bench check-budgets
```

## Architecture

```
crates/
  qndx-core/     Shared types, file format, hashing, file walk, scan-only search
  qndx-index/    Index builder, memory-mapped reader, postings (Vec/Roaring/hybrid)
  qndx-query/    Regex decomposition, query planner, candidate resolution, verification
  qndx-git/      Git integration via gix (dirty detection, HEAD commit)
  qndx-cli/      CLI entrypoints
  qndx-bench/    Benchmark fixtures, report generation, budget checking
```

### Data flow

```
                     build                              search
                     -----                              ------

  source files ──> walk + extract trigrams ──> ngrams.tbl      pattern
                   extract sparse n-grams ──> postings.dat        |
                   collect metadata       ──> manifest.bin        v
                                                           decompose regex
                                                                  |
                                                           plan (trigram vs sparse)
                                                                  |
                                                           lookup n-gram hashes
                                                           intersect posting lists
                                                                  |
                                                           candidate files
                                                                  |
                                                           read + verify (full regex)
                                                                  |
                                                           verified matches
```

### Index files

The index is stored in three files under `.qndx/index/v1/`:

| File | Magic | Contents |
|------|-------|----------|
| `ngrams.tbl` | `QXNG` | Sorted n-gram hash table (20 bytes per entry: hash, offset, length, flags) |
| `postings.dat` | `QXPO` | Concatenated posting blocks (tagged: varint-delta for small lists, Roaring for large) |
| `manifest.bin` | `QXMF` | Metadata and file paths (postcard-serialized) |

Each file has a 20-byte header: 4-byte magic, u32 version, u64 payload length, u32 CRC32 checksum.

See [docs/file-format.md](docs/file-format.md) for the full specification.

## Benchmarking

### Synthetic benchmarks

```bash
# Run all benchmarks
cargo bench

# Run a specific benchmark group
cargo bench -- end_to_end_search
cargo bench -- postings_choice
```

Benchmark targets: `serializer_choice`, `postings_choice`, `ngram_extract`, `query_planner`, `end_to_end_search`, `git_overlay`.

### Real corpus benchmarks

Benchmark against an actual codebase:

```bash
# Basic
QNDX_BENCH_CORPUS=~/src/linux cargo bench --bench real_corpus

# With corpus-specific patterns
QNDX_BENCH_CORPUS=~/src/linux \
QNDX_BENCH_PATTERNS=benchmarks/patterns/linux.txt \
cargo bench --bench real_corpus

# Quick validation (no Criterion iterations)
QNDX_BENCH_CORPUS=~/myproject cargo bench --bench real_corpus -- --test

# Limit files for large repos
QNDX_BENCH_MAX_FILES=5000 \
QNDX_BENCH_CORPUS=~/src/linux cargo bench --bench real_corpus
```

Environment variables:

| Variable | Required | Description |
|----------|----------|-------------|
| `QNDX_BENCH_CORPUS` | Yes | Path to the codebase |
| `QNDX_BENCH_PATTERNS` | No | Path to patterns file (tab-separated `name\tpattern` or one pattern per line) |
| `QNDX_BENCH_NAME` | No | Override corpus name in reports |
| `QNDX_BENCH_MAX_FILES` | No | Limit number of files |
| `QNDX_BENCH_MAX_FILE_SIZE` | No | Override max file size (default: 1 MB) |

HTML reports are generated by Criterion at `target/criterion/real_{name}/report/index.html`.

### Regression tracking

Performance budgets are defined in [`benchmarks/budgets.toml`](benchmarks/budgets.toml). Critical budgets (end-to-end search, postings intersection) fail CI on violation. See [docs/performance-budgets.md](docs/performance-budgets.md) for details.

```bash
# Save a baseline
cargo bench -- --save-baseline main

# Compare against baseline
cargo bench -- --baseline main

# Check budgets
cargo run -p qndx-cli -- bench check-budgets
```

## Development

### Build

```bash
cargo build
cargo build --release
```

### Test

```bash
# All tests (202 tests across all crates)
cargo test --all-features

# Specific crate
cargo test -p qndx-index
cargo test -p qndx-query

# Differential tests (index results == scan results)
cargo test differential

# Regex edge cases
cargo test regex_edge_cases
```

### Lint

```bash
cargo clippy --all-features --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Documentation

| Document | Description |
|----------|-------------|
| [docs/architecture.md](docs/architecture.md) | Crate structure, data flow, design decisions |
| [docs/file-format.md](docs/file-format.md) | On-disk index format specification |
| [docs/decision-gates.md](docs/decision-gates.md) | Benchmark-backed architecture decisions (serializer, postings, n-gram strategy) |
| [docs/performance-budgets.md](docs/performance-budgets.md) | Per-benchmark-group regression thresholds |
| [docs/regression-triage.md](docs/regression-triage.md) | Six-step process for investigating performance regressions |
| [docs/release-gate.md](docs/release-gate.md) | Release criteria and MVP definition of done |

## License

MIT
