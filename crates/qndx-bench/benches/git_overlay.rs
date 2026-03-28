//! Benchmark: git overlay operations
//!
//! Measures gix operations relevant to freshness model.
//! Output: dirty detection/update costs.
//!
//! Note: this benchmark uses stub implementations for M0.
//! Real gix integration will be added in M5.

use criterion::{
    black_box, criterion_group, criterion_main, Criterion,
};
use qndx_git::{detect_dirty_files, head_commit};

fn bench_git_overlay(c: &mut Criterion) {
    let mut group = c.benchmark_group("git_overlay");
    group.sample_size(100);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(2));

    // Benchmark: dirty file detection (stub for now)
    group.bench_function("detect_dirty_files", |b| {
        b.iter(|| {
            let dirty = detect_dirty_files(black_box("."));
            black_box(dirty);
        });
    });

    // Benchmark: HEAD commit lookup (stub for now)
    group.bench_function("head_commit", |b| {
        b.iter(|| {
            let commit = head_commit(black_box("."));
            black_box(commit);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_git_overlay);
criterion_main!(benches);
