//! Benchmark: end-to-end search pipeline
//!
//! Full search pipeline: plan -> candidate generation -> verify.
//! Output: end-to-end latency across query suites.

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion,
};
use qndx_bench::fixtures;
use qndx_index::ngram::extract_trigrams;
use qndx_query::planner::plan_query;
use qndx_query::verify::verify_candidates;

/// Simulated end-to-end search: plan + trigram candidate filter + verify.
fn search_pipeline(pattern: &str, files: &[(String, Vec<u8>)]) -> Vec<usize> {
    let plan = plan_query(pattern);

    // Build per-file trigram sets (in real usage these would be pre-indexed)
    let file_trigrams: Vec<Vec<u32>> = files
        .iter()
        .map(|(_, content)| extract_trigrams(content))
        .collect();

    // Candidate generation: files whose trigrams contain all required grams
    let candidates: Vec<(usize, &[u8])> = files
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let ft = &file_trigrams[*i];
            plan.decomposition
                .required
                .iter()
                .all(|req| ft.binary_search(req).is_ok())
        })
        .map(|(i, (_, content))| (i, content.as_slice()))
        .collect();

    // Verification pass
    verify_candidates(pattern, &candidates)
}

fn bench_end_to_end(c: &mut Criterion) {
    let mut group = c.benchmark_group("end_to_end_search");
    group.sample_size(20);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(5));

    let corpus = fixtures::small_corpus();
    let files: Vec<(String, Vec<u8>)> = corpus
        .files
        .into_iter()
        .map(|f| (f.path, f.content))
        .collect();

    let patterns = fixtures::benchmark_patterns();

    for (name, pattern) in &patterns {
        group.bench_with_input(
            BenchmarkId::new("search", name),
            &(&files, *pattern),
            |b, (files, pattern)| {
                b.iter(|| {
                    let results = search_pipeline(black_box(pattern), black_box(files));
                    black_box(results);
                });
            },
        );
    }

    // Print candidate/match counts for reference
    for (name, pattern) in &patterns {
        let matches = search_pipeline(pattern, &files);
        eprintln!(
            "  [e2e] {}: matches={}/{}",
            name,
            matches.len(),
            files.len(),
        );
    }

    group.finish();
}

criterion_group!(benches, bench_end_to_end);
criterion_main!(benches);
