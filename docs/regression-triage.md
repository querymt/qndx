# Regression Triage Checklist

This document provides a systematic process for investigating, classifying, and resolving performance regressions detected by the automated benchmark system.

---

## When to Use This Checklist

This checklist should be used whenever:

1. CI detects a performance budget violation
2. Manual benchmark comparison shows unexpected performance changes
3. User reports indicate slower performance than previous versions
4. Proactive performance audits reveal concerning trends

---

## Triage Process (Step-by-Step)

### Step 1: Verify the Regression

**Goal**: Confirm the regression is real, not measurement noise or environmental variance.

#### Actions:

- [ ] **Re-run benchmarks locally** in a controlled environment:
  ```bash
  # Run 3 times to check consistency
  cargo bench -- --save-baseline verify-1
  cargo bench -- --save-baseline verify-2
  cargo bench -- --save-baseline verify-3
  
  # Compare against main baseline
  cargo bench -- --baseline main
  ```

- [ ] **Check CI environment consistency**:
  - Was the CI runner consistent (same machine type)?
  - Were there other jobs competing for resources?
  - Check system load/thermal throttling indicators

- [ ] **Statistical significance**:
  - Look at confidence intervals in Criterion output
  - Confirm the change is outside normal variance (typically ±3-5%)
  - Check if p-value indicates statistical significance

#### Decision Point:

- ✅ **Real regression**: Proceed to Step 2
- ❌ **False alarm**: Document noise source, consider adjusting budgets or CI setup
- ⚠️ **Uncertain**: Gather more samples, investigate environment

---

### Step 2: Measure Scope and Impact

**Goal**: Understand which operations are affected and by how much.

#### Actions:

- [ ] **Identify affected benchmarks**:
  - List all benchmark groups showing regression
  - Note the magnitude of each regression (% change)
  - Identify patterns (e.g., all postings operations, only complex queries)

- [ ] **Run targeted profiling**:
  ```bash
  # Profile the regressed benchmark
  cargo bench --bench <benchmark_name> --profile-time 60
  
  # Or use perf/samply for deeper analysis
  perf record -g cargo bench --bench <benchmark_name>
  perf report
  ```

- [ ] **Measure end-to-end impact**:
  - Run `end_to_end_search` benchmark to see user-facing impact
  - Test on representative query workloads
  - Measure both p50 (typical) and p95 (tail) latency

- [ ] **Check resource usage**:
  - Memory consumption changes
  - CPU utilization patterns
  - Disk I/O characteristics
  - Index size changes

#### Output:

Document findings in a structured format:

```markdown
## Regression Scope

- **Affected benchmarks**: postings_choice/intersection/medium, postings_choice/intersection/high
- **Magnitude**: 18% regression in medium cardinality, 22% in high cardinality
- **End-to-end impact**: 12% slower on regex queries, 5% slower on literal queries
- **Resource changes**: +15% memory usage, +8% index size
- **Pattern**: Only affects Roaring bitmap operations, Vec-based postings unaffected
```

---

### Step 3: Analyze Root Cause

**Goal**: Identify the code change or architectural decision that caused the regression.

#### Actions:

- [ ] **Git bisect** to find the introducing commit:
  ```bash
  git bisect start
  git bisect bad HEAD
  git bisect good <last-known-good-commit>
  
  # At each step, run:
  cargo bench -- --baseline main <affected-benchmark>
  git bisect good  # or bad
  ```

- [ ] **Code review** of the culprit commit:
  - What was changed?
  - Was performance considered in the PR review?
  - Are there obvious inefficiencies (unnecessary clones, allocations, etc.)?

- [ ] **Profile comparison** (before vs. after):
  ```bash
  # Checkout before regression
  git checkout <good-commit>
  cargo bench -- --save-baseline before-regression
  
  # Checkout after regression
  git checkout <bad-commit>
  cargo bench -- --save-baseline after-regression
  
  # Profile both
  perf diff before-regression.perf after-regression.perf
  ```

- [ ] **Generate flamegraphs**:
  ```bash
  # Using cargo-flamegraph or samply
  cargo flamegraph --bench <benchmark_name> --open
  ```

- [ ] **Check for common issues**:
  - [ ] Unnecessary allocations or clones
  - [ ] Inefficient data structure usage
  - [ ] Missing inlining hints (`#[inline]`)
  - [ ] Suboptimal algorithm choice
  - [ ] Lock contention or synchronization overhead
  - [ ] Cache-unfriendly access patterns

#### Output:

```markdown
## Root Cause

**Introducing commit**: abc123def "Refactor postings intersection to use generic traits"

**Analysis**:
- Added trait dispatching overhead to postings intersection
- Generic implementation prevents LLVM inlining optimizations
- Monomorphization creates code bloat reducing instruction cache efficiency

**Evidence**:
- Flamegraph shows 15% more time in trait method dispatch
- Perf stat shows 25% increase in instruction cache misses
- Disassembly reveals lack of inlining in hot loop
```

---

### Step 4: Classify the Regression

**Goal**: Determine if the regression is acceptable, requires fixing, or represents a deliberate trade-off.

#### Classification Categories:

##### A. **Bug / Unintended Regression** ❌

The regression provides no benefit and should be fixed.

**Examples**:
- Accidental algorithmic inefficiency
- Missing optimization (inlining, bounds check elimination)
- Unintended allocation in hot path
- Performance-critical code accidentally disabled

**Action**: Fix immediately before merge (or revert if already merged)

---

##### B. **Acceptable Trade-off** ✅

The regression is justified by gains in other areas.

**Examples**:
- 15% slower query but 50% smaller index size (acceptable if within budget)
- 10% slower build but better correctness guarantees
- Small query regression but significantly better freshness/staleness
- Performance for complexity: better maintainability, more features

**Action**: Document trade-off, adjust budgets if necessary, proceed with merge

**Requirements for acceptance**:
- [ ] Trade-off is well-understood and documented
- [ ] Benefits outweigh costs for target use cases
- [ ] Regression stays within adjusted budget thresholds
- [ ] Team consensus on acceptability

---

##### C. **Architecture Change** 🔄

The regression is part of a larger architectural shift.

**Examples**:
- Moving from simple Vec to hybrid postings (temporary regression during transition)
- Refactoring for extensibility that temporarily hurts performance
- Introducing abstractions needed for future features

**Action**: 
- Document temporary nature
- Create follow-up issue to recover performance
- Set timeline for optimization work
- Ensure regression doesn't compound over time

---

##### D. **Acceptable Within Variance** ✓

The regression is within normal measurement variance and budgets.

**Examples**:
- 3% regression on a 10% budget (well within tolerance)
- Non-critical path with generous budget
- Micro-optimization reversal with negligible user impact

**Action**: Monitor but proceed with merge, no changes needed

---

### Step 5: Take Action

**Goal**: Resolve the regression appropriately based on classification.

#### For Bugs (Category A):

- [ ] **Fix the issue**:
  - Optimize hot path
  - Restore inlining
  - Remove unnecessary allocations
  - Revert problematic change if fix is non-trivial

- [ ] **Verify fix**:
  ```bash
  cargo bench -- --baseline main
  # Confirm regression is resolved
  ```

- [ ] **Add regression test**:
  - Add benchmark if one doesn't exist
  - Document expected performance characteristics
  - Update budgets if needed

---

#### For Trade-offs (Category B):

- [ ] **Document decision** in PR description:
  ```markdown
  ## Performance Impact
  
  This PR introduces a 12% regression in query planning time in exchange for:
  - 30% reduction in index size
  - Better sparse n-gram coverage
  - More accurate selectivity estimates
  
  Regression is within the 15% budget for query_planner.
  End-to-end impact is 5% (within 10% budget).
  
  **Triage**: Classified as acceptable trade-off.
  ```

- [ ] **Adjust budgets** if baseline has shifted:
  ```toml
  # benchmarks/budgets.toml
  [query_planner.planning_time]
  p50_regression_pct = 12.0  # Adjusted from 10.0
  p95_regression_pct = 15.0
  # Reason: sparse n-gram planning overhead, acceptable for index size win
  ```

- [ ] **Update decision-gates.md** if architectural choice changes

---

#### For Architecture Changes (Category C):

- [ ] **Create follow-up issue** for performance recovery:
  ```markdown
  ## Issue: Recover performance after postings refactor
  
  **Background**: PR #123 refactored postings to use traits, causing 18% regression.
  This was necessary for hybrid postings implementation.
  
  **Goal**: Recover performance to within 5% of original baseline.
  
  **Approach**:
  - Investigate monomorphization strategies
  - Consider enum-based dispatch instead of traits
  - Profile and optimize hot paths
  
  **Deadline**: Before M4 completion (postings optimization milestone)
  ```

- [ ] **Set timeline** for optimization work

- [ ] **Track regression** in `benchmarks/results/regressions.md`

---

#### For Acceptable Variance (Category D):

- [ ] **Document in commit message** or PR comment

- [ ] **Monitor trend** to ensure variance doesn't compound

---

### Step 6: Document and Close

**Goal**: Create a record for future reference and learning.

#### Actions:

- [ ] **Update `benchmarks/results/regressions.md`**:
  ```markdown
  ## 2026-03-28: Postings Intersection Regression (18%)
  
  **PR**: #123
  **Classification**: Acceptable Trade-off
  **Magnitude**: 18% on medium cardinality, 22% on high cardinality
  **Scope**: Postings intersection operations only
  
  **Root Cause**: Generic trait dispatch overhead
  
  **Decision**: Accepted due to 30% index size reduction and hybrid postings
  enablement. Performance recovery tracked in issue #124.
  
  **Follow-up**: Optimize trait dispatch before M4 milestone
  ```

- [ ] **Update budgets** if baseline has shifted

- [ ] **Share learnings** with team:
  - What was learned?
  - Can we prevent similar regressions?
  - Do we need better tooling or benchmarks?

---

## Quick Reference: Decision Tree

```
┌─────────────────────────────────┐
│ Regression Detected             │
└────────────┬────────────────────┘
             │
             ▼
     ┌───────────────┐
     │ Is it real?   │
     └───┬───────────┘
         │
    ┌────┴────┐
    │         │
   YES       NO ────► Document noise, adjust if needed
    │
    ▼
┌───────────────────┐
│ Measure scope     │
└────────┬──────────┘
         │
         ▼
┌───────────────────┐
│ Find root cause   │
└────────┬──────────┘
         │
         ▼
┌─────────────────────────────────────┐
│ Classify                            │
├─────────────────────────────────────┤
│ A. Bug? ────────────────► FIX       │
│ B. Trade-off? ──────────► DOCUMENT  │
│ C. Architecture? ────────► TRACK    │
│ D. Variance? ────────────► MONITOR  │
└─────────────────────────────────────┘
         │
         ▼
┌─────────────────────┐
│ Document and close  │
└─────────────────────┘
```

---

## Triage Roles

### PR Author
- Responsible for initial investigation
- Provides context on code changes
- Proposes classification and fix

### Reviewer
- Validates triage analysis
- Approves classification
- Ensures documentation is complete

### Maintainer
- Final decision on acceptable regressions
- Budget adjustment approval
- Release gate enforcement

---

## Tools and Commands Reference

### Running Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark group
cargo bench -- postings_choice

# Save baseline
cargo bench -- --save-baseline my-baseline

# Compare against baseline
cargo bench -- --baseline my-baseline

# Generate report
cargo run -p qndx-cli -- bench report
```

### Profiling

```bash
# CPU profiling with perf
perf record -F 999 -g cargo bench --bench <name>
perf report

# Flamegraph
cargo flamegraph --bench <name>

# Memory profiling with valgrind
valgrind --tool=massif cargo bench --bench <name>

# Cache analysis
perf stat -e cache-references,cache-misses,instructions,cycles \
  cargo bench --bench <name>
```

### Git Bisect

```bash
# Start bisect
git bisect start
git bisect bad HEAD
git bisect good v0.1.0

# At each step
cargo bench -- --baseline main <benchmark>
git bisect good  # or bad

# When done
git bisect reset
```

---

## Common Regression Patterns

### Pattern 1: Allocation in Hot Path

**Symptoms**: Increased allocations, slower throughput

**Fix**: Use stack allocation, reuse buffers, `SmallVec`

**Example**:
```rust
// Before: allocates on every call
fn process(items: &[Item]) -> Vec<Result> {
    items.iter().map(|i| i.process()).collect()
}

// After: reuse buffer
fn process(items: &[Item], out: &mut Vec<Result>) {
    out.clear();
    for i in items {
        out.push(i.process());
    }
}
```

---

### Pattern 2: Missing Inlining

**Symptoms**: Increased call overhead, especially in small functions

**Fix**: Add `#[inline]` or `#[inline(always)]`

**Example**:
```rust
#[inline]
fn is_separator(c: char) -> bool {
    c.is_whitespace() || c == '_'
}
```

---

### Pattern 3: Trait Dispatch Overhead

**Symptoms**: Slower than monomorphic code, hard to optimize

**Fix**: Use generics with trait bounds, or enum dispatch

**Example**:
```rust
// Slower: trait object dispatch
fn intersect(a: &dyn Postings, b: &dyn Postings) -> Vec<u32>

// Faster: generic monomorphization
fn intersect<A: Postings, B: Postings>(a: &A, b: &B) -> Vec<u32>
```

---

### Pattern 4: Cache-Unfriendly Access

**Symptoms**: High cache miss rate, poor memory bandwidth

**Fix**: Improve data locality, use SoA instead of AoS

**Example**:
```rust
// Bad: AoS with poor locality
struct Entry { id: u32, data: [u8; 1024] }
let entries: Vec<Entry>;

// Good: SoA with better locality  
struct Entries { ids: Vec<u32>, data: Vec<[u8; 1024]> }
```

---

## Escalation Path

If triage is stuck or contentious:

1. **Discuss in PR comments**: Get initial feedback
2. **Team sync**: Bring to next team meeting if complex
3. **Benchmark sprint**: Dedicate focused time to investigation
4. **Technical decision**: Document in ADR (Architecture Decision Record)

---

## Continuous Improvement

After each triage:

- [ ] Did the checklist help?
- [ ] Are budgets appropriate?
- [ ] Do we need better benchmarks?
- [ ] Should we adjust the process?

Update this document based on lessons learned.
