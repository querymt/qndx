# Performance Budgets

This document defines explicit performance budgets for each benchmark group to enable automated regression detection in CI. Each budget includes tracked metrics, threshold values, and rationale.

---

## Overview

Performance budgets are defined per benchmark group and represent the maximum acceptable regression from the baseline. Exceeding these thresholds will fail CI and require investigation before merge.

### General Principles

- **Conservative thresholds**: Set high enough to catch real regressions, low enough to avoid false positives from noise
- **Environment-aware**: Assume benchmarks run in CI with typical variance (~3-5%)
- **Differentiated by criticality**: End-to-end user-facing operations have tighter budgets than internal operations

---

## Tracked Metrics

The following metrics are tracked across all benchmark groups:

1. **Build throughput (MB/s)**: How fast we can index corpus data
2. **Index size (% of corpus)**: Overhead of index storage relative to source files
3. **p50/p95 query latency by query class**: Median and tail latency for different query patterns
4. **Candidate counts before verify**: Precision of candidate generation (lower is better)
5. **Verify stage share of total time**: Percentage of query time spent in verification

---

## Benchmark Groups and Budgets

### 1. `serializer_choice`

**Purpose**: Compare manifest serialization strategies (postcard, wincode, serde_json)

| Metric | Budget | Rationale |
|--------|--------|-----------|
| Encode throughput | ±20% | Non-critical path, infrequent operation |
| Decode throughput | ±20% | Non-critical path, happens at index open |
| Encoded size | ±15% | Manifests are small, size impact is minimal |

**Failure threshold**: None (informational benchmark for architecture decisions)

---

### 2. `postings_choice`

**Purpose**: Compare postings representations (Vec, Roaring, hybrid) and operations

| Metric | Budget | Rationale |
|--------|--------|-----------|
| Intersection latency (low cardinality) | **±15%** | Core query operation, frequent |
| Intersection latency (medium cardinality) | **±15%** | Core query operation, frequent |
| Intersection latency (high cardinality) | **±15%** | Core query operation, frequent |
| Union latency (low cardinality) | ±20% | Less frequent than intersection |
| Union latency (medium cardinality) | ±20% | Less frequent than intersection |
| Union latency (high cardinality) | ±20% | Less frequent than intersection |
| Encode throughput | ±25% | Non-critical path, happens at index build |
| Decode throughput | ±20% | Happens at query time but one-time cost |
| Encoded size | ±15% | Impacts index size budget |

**Failure threshold**: >15% regression in any intersection operation

---

### 3. `ngram_extract`

**Purpose**: Compare n-gram extraction strategies (trigram, sparse)

| Metric | Budget | Rationale |
|--------|--------|-----------|
| Extraction throughput (MB/s) | ±20% | Happens at index build, less critical than query |
| Grams produced (count) | ±30% | Expected to vary with strategy changes |
| Estimated index size | **±25%** | Impacts disk usage, must stay in budget |

**Failure threshold**: >25% increase in estimated index size

---

### 4. `query_planner`

**Purpose**: Measure query decomposition and planning overhead

| Metric | Budget | Rationale |
|--------|--------|-----------|
| Planning time (p50) | **±10%** | Happens per query, user-facing |
| Planning time (p95) | **±15%** | Tail latency matters for interactive use |
| Postings lookups (count) | **±20%** | Fewer lookups = faster query execution |
| Candidate set size estimate | ±30% | Precision varies by query, verified anyway |

**Failure threshold**: >10% regression in p50 planning time or >20% increase in postings lookups

---

### 5. `end_to_end_search`

**Purpose**: Full search pipeline (plan -> postings -> verify) — **most critical benchmark**

| Metric | Budget | Rationale |
|--------|--------|-----------|
| Search latency p50 (literal queries) | **±10%** | Critical user-facing path |
| Search latency p95 (literal queries) | **±12%** | Tail latency for common case |
| Search latency p50 (regex queries) | **±10%** | Critical user-facing path |
| Search latency p95 (regex queries) | **±15%** | More variance in regex complexity |
| Search latency p50 (complex queries) | **±12%** | Advanced use case but still important |
| Search latency p95 (complex queries) | **±18%** | Higher variance acceptable |
| Candidate count (pre-verify) | ±30% | False positive rate, verified anyway |
| Verify stage % of total time | ±25% | Balance between candidate precision and verify cost |
| Matches returned (correctness) | **0% tolerance** | No false negatives or positives |

**Failure threshold**: >10% regression in p50 search latency for literal or regex queries

**Correctness requirement**: Match counts must be identical to baseline (differential test)

---

### 6. `git_overlay`

**Purpose**: Git operations for freshness (dirty detection, overlay updates)

| Metric | Budget | Rationale |
|--------|--------|-----------|
| Dirty file detection time | **±15%** | Happens at query time for fresh results |
| Overlay update throughput (MB/s) | ±20% | Proportional to working tree size |
| Overlay query merge time | **±10%** | Happens per query, user-facing |

**Failure threshold**: >15% regression in dirty detection or >10% regression in overlay merge

---

## Build and Index Metrics

These metrics are tracked across the entire index build process:

| Metric | Budget | Rationale |
|--------|--------|-----------|
| Overall build throughput (MB/s) | ±20% | Batch operation, less critical than query |
| Total index size (% of corpus) | **±25%** | Must stay under ~50% of corpus size |
| Trigram count | ±30% | Implementation detail, informational |
| Sparse n-gram count | ±40% | Strategy-dependent, informational |
| Total n-gram count | **±25%** | Drives index size |

**Failure threshold**: >25% increase in total index size relative to corpus

---

## Query Classification and Coverage

Queries are categorized into classes for targeted budgets:

1. **Literal queries**: Simple substring search (e.g., `"error"`, `"TODO"`)
   - Tightest budget: ±10% p50, ±12% p95
   - Most common use case

2. **Regex queries**: Pattern matching (e.g., `"err.*log"`, `"\b[A-Z]+\b"`)
   - Standard budget: ±10% p50, ±15% p95
   - Common for code search

3. **Complex queries**: Multi-term, nested patterns (e.g., `"(foo|bar).*baz"`)
   - Relaxed budget: ±12% p50, ±18% p95
   - Less frequent, more variable

4. **Pathological queries**: Edge cases (e.g., `".*"`, `"a+"`)
   - No strict budget, must not hang or OOM
   - Handled by timeouts and candidate limits

---

## CI Integration

### Regression Detection Process

1. **Baseline**: Store benchmark results from `main` branch as named baseline
   ```bash
   cargo bench -- --save-baseline main
   ```

2. **PR Comparison**: Run benchmarks on PR branch and compare against baseline
   ```bash
   cargo bench -- --baseline main
   ```

3. **Threshold Check**: Parse Criterion output and compare against budgets defined above

4. **Failure Modes**:
   - **Hard fail**: Exceeds critical end-to-end or postings budgets (>10-15%)
   - **Soft fail (warning)**: Exceeds non-critical budgets, requires justification
   - **Correctness fail**: Match count mismatch (immediate block)

5. **Artifact Storage**: Save Criterion JSON results to `benchmarks/results/` for trend analysis
   ```bash
   # Results stored automatically by Criterion
   target/criterion/<benchmark_name>/*/estimates.json
   target/criterion/<benchmark_name>/*/sample.json
   ```

### Example CI Workflow Steps

```yaml
- name: Run benchmarks
  run: cargo bench -- --save-baseline pr-${{ github.event.pull_request.number }}

- name: Compare against main baseline
  run: cargo bench -- --baseline main --output-format json > bench-comparison.json

- name: Check performance budgets
  run: cargo run -p qndx-cli -- bench check-budgets bench-comparison.json

- name: Upload benchmark results
  uses: actions/upload-artifact@v3
  with:
    name: benchmark-results
    path: target/criterion/
```

---

## Triage Process

When a regression is detected:

1. **Verify**: Re-run locally to rule out CI noise/variance
2. **Measure scope**: Which queries/operations are affected? By how much?
3. **Analyze root cause**: Profile the regression (flamegraph, perf stat)
4. **Classify**:
   - **Acceptable**: Justified by feature gain or architecture improvement, document in PR
   - **Bug**: Fix before merge
   - **Trade-off**: Discuss with team, potentially adjust budget if new baseline is reasonable

5. **Document**: Update `benchmarks/results/regressions.md` with triage notes

---

## Budget Adjustments

Budgets may be adjusted when:

1. **Corpus characteristics change**: New benchmark fixtures with different properties
2. **Architecture decisions**: Deliberate trade-offs (e.g., more grams for better recall)
3. **Baseline shift**: After a major refactor with new performance characteristics
4. **Measurement improvements**: Better benchmark design reveals real variance

**Process**: Propose budget adjustment in PR with:
- Justification (why old budget is no longer appropriate)
- Supporting benchmark data
- Team review and approval

---

## Expected Performance Ranges (Baseline)

These are rough target ranges for the initial MVP baseline (to be updated with real measurements):

### Index Build
- Throughput: 50-200 MB/s (varies by file type, n-gram strategy)
- Index size: 20-50% of corpus size (target: <50%)

### Query Performance (medium corpus ~100MB, ~1000 files)
- Literal query p50: 1-10ms
- Literal query p95: 5-50ms
- Regex query p50: 5-30ms
- Regex query p95: 20-100ms

### Git Overlay
- Dirty detection: 1-20ms (depends on working tree size)
- Overlay merge: <5% of total query time

---

## Machine-Readable Budget Format

For CI integration, budgets are also stored in TOML:

```toml
# benchmarks/budgets.toml
[postings_choice.intersection.low]
p50_regression_pct = 15.0
p95_regression_pct = 15.0

[postings_choice.intersection.medium]
p50_regression_pct = 15.0
p95_regression_pct = 15.0

[end_to_end_search.literal]
p50_regression_pct = 10.0
p95_regression_pct = 12.0
critical = true

[end_to_end_search.regex]
p50_regression_pct = 10.0
p95_regression_pct = 15.0
critical = true

[ngram_extract]
index_size_growth_pct = 25.0
critical = true
```

---

## Summary

- **Critical budgets**: End-to-end search (±10%), postings intersection (±15%), index size (±25%)
- **Non-critical budgets**: Build throughput (±20%), serialization (±20%), planning (±10%)
- **Zero tolerance**: Correctness (match counts must be exact)
- **CI enforcement**: Hard fail on critical budget violations, warning on non-critical
- **Trend tracking**: Store all results in `benchmarks/results/` for historical analysis

These budgets balance catching real regressions while allowing for CI variance and non-critical optimizations.
