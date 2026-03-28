//! Benchmark: query planner
//!
//! Measures decomposition + lookup count estimation.
//! Output: candidate set size, postings lookups, planning time.

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion,
};
use qndx_bench::fixtures;
use qndx_query::planner::plan_query;

fn bench_query_planner(c: &mut Criterion) {
    let mut group = c.benchmark_group("query_planner");
    group.sample_size(200);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(3));

    let patterns = fixtures::benchmark_patterns();

    for (name, pattern) in &patterns {
        group.bench_with_input(
            BenchmarkId::new("plan", name),
            pattern,
            |b, p| {
                b.iter(|| {
                    let plan = plan_query(black_box(p));
                    black_box(plan);
                });
            },
        );
    }

    // Print plan summaries for reference
    for (name, pattern) in &patterns {
        let plan = plan_query(pattern);
        eprintln!(
            "  [plan] {}: lookups={}, required_grams={}",
            name, plan.lookup_count, plan.decomposition.required.len(),
        );
    }

    group.finish();
}

criterion_group!(benches, bench_query_planner);
criterion_main!(benches);
