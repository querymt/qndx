# Decision Gates

This document defines the three explicit decision gates for qndx architecture choices.
Each gate has measurable criteria, a default winner policy, and instructions for
evaluating the decision using the benchmark harness.

---

## Gate A: Manifest Serializer

**Candidates:** postcard vs wincode (vs optional tiny custom manifest)

### Criteria

| Metric | Threshold |
|--------|-----------|
| Decode throughput | wincode >= 15% better on manifest-heavy workloads |
| Compatibility | No maintenance or compatibility concerns |
| Encode size | Within 10% of postcard |

### Default Winner Policy

- If wincode provides >= 15% better decode throughput on manifest-heavy workloads
  **and** no compatibility/maintenance concerns arise, choose wincode.
- Otherwise choose **postcard** for simplicity and stable wire format.

### How to Evaluate

```bash
cargo bench -- serializer_choice
```

Compare `manifest_encode` and `manifest_decode` groups across postcard and wincode.
Record median throughput and encoded sizes from Criterion output.

### Current Decision

**postcard** (default). Re-evaluate when `benches/serializer_choice.rs` shows a
clear wincode advantage at realistic manifest sizes (1K--10K files).

---

## Gate B: Postings Representation

**Candidates:** Vec<u32> (delta-encoded), Roaring bitmap, hybrid (threshold-based)

### Criteria

| Metric | Threshold |
|--------|-----------|
| End-to-end query latency | Hybrid >= 10% faster than single-format |
| Memory / index size | Does not exceed budget (< 2x baseline) |
| Intersection/union op latency | No regression on core operations |

### Default Winner Policy

- Choose **hybrid** when it is >= 10% faster end-to-end query latency than
  single-format options and memory/index size does not exceed budget.
- Otherwise choose Vec<u32> delta-encoded for simplicity.

### How to Evaluate

```bash
cargo bench -- postings_choice
```

Compare `intersect` and `union` groups across `vec_delta`, `roaring`, and `hybrid`
at low/medium/high cardinality. Measure memory footprint using the reported index
size from `build_index`.

### Current Decision

**Hybrid** with threshold at 64 entries (implemented in `qndx-index/src/postings.rs`).
Re-evaluate if index size grows beyond budget.

---

## Gate C: Sparse vs Trigram Default

**Candidates:** pure trigram decomposition vs sparse n-gram covering

### Criteria

| Metric | Threshold |
|--------|-----------|
| Median postings lookups | Sparse drops >= 25% vs trigram |
| p95 query latency | Improves on medium+ corpus |
| Index-size growth | Remains within budget (< 50% growth) |
| Correctness | No false negatives (differential tests pass) |

### Default Winner Policy

- Choose **sparse as default** when:
  - Median postings lookups drop >= 25% across target query classes
  - p95 query latency improves on medium+ corpus
  - Index-size growth remains within agreed budget
- Otherwise keep **trigram** as default.

### How to Evaluate

```bash
# Compare n-gram extraction build costs
cargo bench -- ngram_extract

# Compare planner lookup counts (printed to stderr)
cargo bench -- query_planner

# Compare end-to-end search latency
cargo bench -- end_to_end_search
```

The planner benchmark prints a table comparing strategy selection, lookup counts,
and costs for each benchmark pattern. The end-to-end benchmark reports which
strategy was selected and match counts.

To measure index-size impact:

```rust
let result = build_index(&files, &index_dir, None)?;
println!("trigrams: {}, sparse: {}, total: {}",
    result.trigram_count, result.sparse_count, result.ngram_count);
```

### Current Decision

**Planner-selected** (implemented). The `QueryPlan` evaluates both strategies
using selectivity estimates and picks the lower-cost option per query. This
means sparse is used when beneficial and trigrams are used as fallback.

Key implementation details:
- `HashSelectivity`: default estimator, assumes longer grams are more selective
  (cost = 3.0 / gram_len)
- `FrequencySelectivity`: optional estimator using actual document-frequency counts
- Sparse covering only wins when it produces fewer lookups than the trigram path
- All results verified by `no_false_negatives_vs_scan` differential tests

### Recording Decisions

After running benchmarks, record results in `benchmarks/results/` as JSON:

```bash
cargo bench -- --save-baseline gate-c-eval
```

Document the decision with:
1. Benchmark environment (CPU, OS, corpus size)
2. Median and p95 latencies for trigram vs sparse across query classes
3. Index size comparison
4. Conclusion and rationale

---

## Evaluation Process

1. Run the relevant benchmark suite against the current baseline
2. Compare results using Criterion's built-in comparison
3. Record decision with supporting data in this document or `benchmarks/results/`
4. Update the default configuration if a gate threshold is met
5. Re-evaluate when the codebase or corpus characteristics change significantly

### Regression Policy

- Every performance-impacting PR runs benchmark subset against baseline
- Fail CI on threshold breaches:
  - > 10% regression in critical end-to-end benches
  - > 15% regression in core postings operations
- Store trend artifacts in `benchmarks/results/*.json`
