//! Benchmark: query planner
//!
//! Measures decomposition + lookup count estimation for both trigram and sparse strategies.
//! Output: candidate set size, postings lookups, planning time, strategy selection.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use qndx_bench::fixtures;
use qndx_query::planner::plan_query;
use std::hint::black_box;

fn bench_query_planner(c: &mut Criterion) {
    let mut group = c.benchmark_group("query_planner");
    group.sample_size(200);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(3));

    let patterns = fixtures::benchmark_patterns();

    for (name, pattern) in &patterns {
        group.bench_with_input(BenchmarkId::new("plan", name), pattern, |b, p| {
            b.iter(|| {
                let plan = plan_query(black_box(p));
                black_box(plan);
            });
        });
    }

    // Print plan summaries for reference (sparse vs trigram comparison)
    eprintln!();
    eprintln!(
        "  {:<25} {:>8} {:>8} {:>8} {:>10} {:>10}",
        "pattern", "strategy", "lookups", "tri_grams", "spr_grams", "cost"
    );
    eprintln!("  {}", "-".repeat(75));
    for (name, pattern) in &patterns {
        let plan = plan_query(pattern);
        eprintln!(
            "  {:<25} {:>8?} {:>8} {:>8} {:>10} {:>10.3}",
            name,
            plan.strategy,
            plan.lookup_count,
            plan.decomposition.required.len(),
            plan.decomposition.sparse_required.len(),
            plan.estimated_cost,
        );
    }

    group.finish();
}

criterion_group!(benches, bench_query_planner);
criterion_main!(benches);
