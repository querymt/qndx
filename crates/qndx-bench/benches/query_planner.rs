//! Benchmark: query planner
//!
//! Measures decomposition + lookup count estimation for both trigram and sparse strategies.
//! Output: candidate set size, postings lookups, planning time, strategy selection.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use qndx_bench::fixtures;
use qndx_query::decompose::decompose_pattern;
use qndx_query::planner::{plan_diagnostics, plan_query};
use std::hint::black_box;

fn bench_query_planner(c: &mut Criterion) {
    let mut group = c.benchmark_group("query_planner");
    group.sample_size(200);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(3));

    let patterns = fixtures::benchmark_patterns();

    for (name, pattern) in &patterns {
        group.bench_with_input(BenchmarkId::new("decompose", name), pattern, |b, p| {
            b.iter(|| {
                let decomposition = decompose_pattern(black_box(p));
                black_box(decomposition);
            });
        });

        group.bench_with_input(BenchmarkId::new("plan", name), pattern, |b, p| {
            b.iter(|| {
                let plan = plan_query(black_box(p));
                black_box(plan);
            });
        });

        group.bench_with_input(BenchmarkId::new("diagnostics", name), pattern, |b, p| {
            b.iter(|| {
                let diag = plan_diagnostics(black_box(p));
                black_box(diag);
            });
        });
    }

    // Print plan summaries for reference (sparse vs trigram comparison)
    eprintln!();
    eprintln!(
        "  {:<25} {:>8} {:>8} {:>8} {:>8} {:>10} {:>10}",
        "pattern", "strategy", "lookups", "tri", "spr", "tot_cost", "sel_cost"
    );
    eprintln!("  {}", "-".repeat(90));
    for (name, pattern) in &patterns {
        let plan = plan_query(pattern);
        let diag = plan_diagnostics(pattern);
        let sel_cost = match plan.strategy {
            qndx_query::planner::PlanStrategy::Trigram => diag.trigram_selectivity_cost,
            qndx_query::planner::PlanStrategy::Sparse => {
                diag.sparse_selectivity_cost.unwrap_or(0.0)
            }
        };
        eprintln!(
            "  {:<25} {:>8?} {:>8} {:>8} {:>8} {:>10.3} {:>10.3}",
            name,
            plan.strategy,
            plan.lookup_count,
            plan.decomposition.required.len(),
            plan.decomposition.sparse_required.len(),
            plan.estimated_cost,
            sel_cost,
        );
    }

    group.finish();
}

criterion_group!(benches, bench_query_planner);
criterion_main!(benches);
