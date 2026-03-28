//! Benchmark: end-to-end search pipeline
//!
//! Full search pipeline: plan -> candidate generation -> verify.
//! Output: end-to-end latency across query suites.
//! Compares trigram-only vs planner-selected (potentially sparse) strategy.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use qndx_bench::fixtures;
use qndx_index::ngram::{extract_sparse_ngrams, extract_trigrams};
use qndx_query::planner::{plan_query, PlanStrategy};
use qndx_query::verify::verify_candidates;
use std::hint::black_box;

/// Build per-file n-gram lookup tables (both trigram and sparse).
struct FileNgrams {
    trigrams: Vec<Vec<u32>>,
    /// Sparse n-gram hashes per file (sorted).
    sparse: Vec<Vec<u32>>,
}

impl FileNgrams {
    fn build(files: &[(String, Vec<u8>)]) -> Self {
        let trigrams = files
            .iter()
            .map(|(_, content)| extract_trigrams(content))
            .collect();
        let sparse = files
            .iter()
            .map(|(_, content)| {
                let mut hashes: Vec<u32> = extract_sparse_ngrams(content)
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                hashes.sort_unstable();
                hashes.dedup();
                hashes
            })
            .collect();
        FileNgrams { trigrams, sparse }
    }
}

/// End-to-end search using the planner-selected strategy.
fn search_pipeline(
    pattern: &str,
    files: &[(String, Vec<u8>)],
    file_ngrams: &FileNgrams,
) -> Vec<usize> {
    let plan = plan_query(pattern);

    // Candidate generation: files whose n-grams contain all required hashes
    let ngram_table = match plan.strategy {
        PlanStrategy::Trigram => &file_ngrams.trigrams,
        PlanStrategy::Sparse => &file_ngrams.sparse,
    };

    let candidates: Vec<(usize, &[u8])> = files
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let ft = &ngram_table[*i];
            plan.required_hashes
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

    let file_ngrams = FileNgrams::build(&files);
    let patterns = fixtures::benchmark_patterns();

    for (name, pattern) in &patterns {
        group.bench_with_input(
            BenchmarkId::new("search", name),
            &(&files, &file_ngrams, *pattern),
            |b, (files, file_ngrams, pattern)| {
                b.iter(|| {
                    let results = search_pipeline(
                        black_box(pattern),
                        black_box(files),
                        black_box(file_ngrams),
                    );
                    black_box(results);
                });
            },
        );
    }

    // Print candidate/match counts and strategy for reference
    eprintln!();
    eprintln!(
        "  {:<25} {:>8} {:>8} {:>8}",
        "pattern", "strategy", "matches", "total"
    );
    eprintln!("  {}", "-".repeat(55));
    for (name, pattern) in &patterns {
        let plan = plan_query(pattern);
        let matches = search_pipeline(pattern, &files, &file_ngrams);
        eprintln!(
            "  {:<25} {:>8?} {:>8} {:>8}",
            name,
            plan.strategy,
            matches.len(),
            files.len(),
        );
    }

    group.finish();
}

criterion_group!(benches, bench_end_to_end);
criterion_main!(benches);
