# qndx MVP Plan (Conscious + Benchmark-Driven)

## 0) Why this plan exists

This plan is intentionally benchmark-first. We want to avoid premature architecture lock-in and instead use repeatable Criterion benches to choose implementation details (data layout, serialization strategy, and index/query tradeoffs) while preventing performance regressions over time.

The MVP target is a local regex search indexer for large repositories with these properties:

- faster interactive query latency than full-repo scan for common agent-style regex queries
- no false negatives (final verification pass over file content)
- fresh results for local edits (read-your-writes behavior)
- stable enough file formats to iterate without painful rewrites

---

## 1) Product and technical scope

### In scope (MVP)

- local index built from repository files
- candidate generation via n-gram index (trigram baseline, then sparse n-grams)
- deterministic regex verification on candidate files
- git-aware freshness model (baseline at commit + working tree overlay)
- benchmark harness with decision gates and regression checks

### Out of scope (MVP)

- distributed/server search
- semantic/code-intel ranking
- full-text ranking quality beyond simple usefulness heuristics
- perfect incremental compaction strategy (a simple working overlay is enough)

---

## 2) Architecture hypotheses (to validate with benchmarks)

1. **Core storage** should be custom binary for hot path (`ngrams.tbl` + `postings.dat`).
2. **Postings representation** should likely be hybrid (`Vec<u32>` for small sets, Roaring for large sets).
3. **Sparse n-grams** should reduce query-time posting lookups versus pure trigrams at acceptable index-size overhead.
4. **Manifest/control metadata** can be serialized with postcard or wincode; pick by measured result + maintenance risk.
5. **gix** is the default Git integration path (with optional adapter fallback if needed).

---

## 3) Project structure

Recommended workspace layout:

- `crates/qndx-cli` - CLI entrypoints (index/search/bench/report)
- `crates/qndx-core` - shared types, file format defs, hashing, IDs
- `crates/qndx-index` - index builder, storage writer/reader
- `crates/qndx-query` - regex decomposition, candidate planner, verifier
- `crates/qndx-git` - git integration (gix-first)
- `crates/qndx-bench` - shared bench fixtures/utilities
- `benches/` - Criterion benchmark targets

Data files (versioned):

- `index/v1/ngrams.tbl` (sorted hash -> offset/len/flags)
- `index/v1/postings.dat` (concatenated postings blocks)
- `index/v1/manifest.bin` (small metadata)

---

## 4) Milestones and acceptance criteria

## M0 - Benchmark foundation first

### Deliverables

- Criterion integrated and running in CI/local
- deterministic fixture datasets
- benchmark report script (human-readable + machine-readable)

### Tasks

- add Criterion and baseline benchmark targets
- create fixture strategy:
  - synthetic corpus (identifier-heavy, random-ish, mixed file sizes)
  - real corpus snapshots (small, medium, large)
- lock benchmark environment knobs (thread count, warmups, sample size)

### Acceptance

- `cargo bench` runs reliably
- baseline artifacts are produced and comparable between commits

---

## M1 - Correctness baseline (scan-only search)

### Deliverables

- search path that scans files directly (no index)
- correctness oracle against expected regex behavior

### Tasks

- file discovery and ignore handling
- regex execution + match extraction
- test suite for tricky regex cases (alternation, classes, escapes)

### Acceptance

- deterministic output and tests pass
- baseline latency numbers recorded for comparison

---

## M2 - Trigram index v0 (functional)

### Deliverables

- trigram index writer + reader
- candidate set generation from decomposed query literals
- verify pass over candidate files

### Tasks

- index build for overlapping trigrams
- sorted lookup table and postings file format
- binary search lookup and postings intersection/union

### Acceptance

- no false negatives against scan-only baseline
- query latency improvement on medium+ corpus for literal/regex subsets

---

## M3 - Sparse n-grams and planner optimization

### Deliverables

- sparse n-gram extraction (build-all)
- sparse covering at query time (minimal lookup set)
- selectivity-aware planner

### Tasks

- deterministic weight function support:
  - initial hash-based weights
  - optional pair-frequency table mode
- planner chooses lower-cost gram set

### Acceptance

- fewer posting lookups than trigram baseline for target query classes
- measurable end-to-end query win without unacceptable index bloat

---

## M4 - Postings optimization and serialization decisions

### Deliverables

- hybrid postings representation
- manifest serialization choice finalized via benchmark gate

### Tasks

- implement and benchmark:
  - `Vec<u32>` delta/varint path
  - Roaring path
  - hybrid threshold policy
- benchmark metadata serializers:
  - postcard
  - wincode
  - (optional) tiny custom manifest

### Acceptance

- winning policy chosen and documented
- decision backed by reproducible benchmark outputs

---

## M5 - Freshness model (Git baseline + overlay)

### Deliverables

- index state pinned to commit
- working-tree/untracked overlay updates
- merged query path (baseline + overlay)

### Tasks

- integrate `gix` for commit/worktree state
- update dirty-file mini-index quickly
- fallback behavior for transient inconsistencies

### Acceptance

- read-your-writes behavior verified
- update-to-query latency meets interactive target

---

## M6 - Regression tracking and release criteria

### Deliverables

- automated perf regression checks
- reproducible benchmark summary in repository artifacts

### Tasks

- define performance budgets per benchmark group
- compare against saved Criterion baseline each PR/commit
- add regression triage checklist

### Acceptance

- regressions are detected automatically
- release gate includes correctness + perf budgets

---

## 5) Criterion benchmark matrix (decision-focused)

Use dedicated bench targets so each architectural decision has data.

1. `benches/serializer_choice.rs`
   - compare postcard vs wincode (and optional custom manifest)
   - workload: manifest encode/decode at realistic sizes and frequencies
   - output: latency, throughput, encoded size

2. `benches/postings_choice.rs`
   - compare vec-delta, roaring, hybrid
   - workload: intersection/union over low/medium/high cardinality postings
   - output: query op latency + memory footprint

3. `benches/ngram_extract.rs`
   - trigram vs sparse build cost
   - output: build throughput, grams produced, index-size estimate

4. `benches/query_planner.rs`
   - decomposition + lookup count estimate
   - output: candidate set size, postings lookups, planning time

5. `benches/end_to_end_search.rs`
   - full search pipeline (plan -> postings -> verify)
   - output: end-to-end latency across query suites

6. `benches/git_overlay.rs`
   - gix operations relevant to freshness
   - output: dirty detection/update costs

---

## 6) Decision gates (explicit)

### Gate A: manifest serializer

- default winner policy:
  - if wincode provides >=15% better decode throughput on manifest-heavy workloads **and** no compatibility/maintenance concerns arise, choose wincode
  - otherwise choose postcard for simplicity and stable wire format

### Gate B: postings representation

- choose hybrid when:
  - hybrid is >=10% faster end-to-end query latency than single-format options
  - and memory/index size does not exceed budget

### Gate C: sparse vs trigram default

- choose sparse default when:
  - median postings lookups drop >=25%
  - p95 query latency improves on medium+ corpus
  - index-size growth remains within agreed budget

---

## 7) Regression policy

### What to track

- build throughput (MB/s)
- index size (% of corpus)
- p50/p95 query latency by query class
- candidate counts before verify
- verify stage share of total time

### Process

- maintain named Criterion baselines (e.g., `main`, `release-X`)
- every perf-impacting PR runs benchmark subset against baseline
- fail CI on threshold breaches (example):
  - >10% regression in critical end-to-end benches
  - >15% regression in core postings operations
- store trend artifacts (`benchmarks/results/*.json`) for history

### Commands (example flow)

- save baseline:
  - `cargo bench -- --save-baseline main`
- compare against baseline:
  - `cargo bench -- --baseline main`
- generate summarized report:
  - `cargo run -p qndx-cli -- bench report`

---

## 8) Correctness and safety checks

- property tests for decomposition and postings operations
- differential tests: index-backed results must match scan-only results
- corpus integrity checksums for reproducibility
- file-format versioning with magic/version/checksum headers

---

## 9) MVP definition of done

MVP is done when all are true:

1. index-backed search is correctness-equivalent to scan baseline (no false negatives)
2. measured query latency wins on target corpora/query sets
3. Git overlay freshness works for local edits
4. performance baselines and regression checks are automated
5. format versioning and migration strategy are documented

---

## 10) Immediate next actions

1. set up workspace skeleton and `Criterion` harness (`M0`)
2. implement scan-only correctness oracle (`M1`)
3. ship trigram index v0 and capture first end-to-end baseline (`M2`)
4. start sparse n-gram experiment branch with controlled benchmarks (`M3`)

---

## 11) Notes to keep this plan honest

- no architecture decision is final until benchmark-backed
- optimize only measured bottlenecks
- preserve simple data formats unless complexity pays for itself in measured wins
- prefer reproducible benchmark methodology over one-off local wins
