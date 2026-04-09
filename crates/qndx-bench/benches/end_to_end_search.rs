//! Benchmark: end-to-end search pipeline
//!
//! Full search pipeline: plan -> candidate generation -> verify.
//! Output: end-to-end latency across query suites.
//! Compares trigram-only vs planner-selected (potentially sparse) strategy.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use qndx_bench::fixtures;
use qndx_index::ngram::{extract_sparse_ngrams_all, extract_trigrams};
use qndx_query::planner::{PlanStrategy, QueryPlan, plan_query};
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
                let mut hashes: Vec<u32> = extract_sparse_ngrams_all(content)
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

fn candidate_ids_from_plan(plan: &QueryPlan, file_ngrams: &FileNgrams) -> Vec<usize> {
    let ngram_table = match plan.strategy {
        PlanStrategy::Trigram => &file_ngrams.trigrams,
        PlanStrategy::Sparse => &file_ngrams.sparse,
    };

    ngram_table
        .iter()
        .enumerate()
        .filter(|(_, ft)| {
            plan.required_hashes
                .iter()
                .all(|req| ft.binary_search(req).is_ok())
        })
        .map(|(i, _)| i)
        .collect()
}

fn candidate_slices<'a>(ids: &[usize], files: &'a [(String, Vec<u8>)]) -> Vec<(usize, &'a [u8])> {
    ids.iter()
        .copied()
        .map(|i| (i, files[i].1.as_slice()))
        .collect()
}

fn search_pipeline_with_plan(
    pattern: &str,
    files: &[(String, Vec<u8>)],
    file_ngrams: &FileNgrams,
    plan: &QueryPlan,
) -> Vec<usize> {
    let candidate_ids = candidate_ids_from_plan(plan, file_ngrams);
    let candidates = candidate_slices(&candidate_ids, files);
    verify_candidates(pattern, &candidates)
}

/// End-to-end search using the planner-selected strategy.
fn search_pipeline(
    pattern: &str,
    files: &[(String, Vec<u8>)],
    file_ngrams: &FileNgrams,
) -> Vec<usize> {
    let plan = plan_query(pattern);
    search_pipeline_with_plan(pattern, files, file_ngrams, &plan)
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
        // Existing benchmark used by CI budgets.
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

        // Diagnostic stage: planning only.
        group.bench_with_input(BenchmarkId::new("plan_only", name), pattern, |b, p| {
            b.iter(|| {
                let plan = plan_query(black_box(p));
                black_box(plan);
            });
        });

        let preplan = plan_query(pattern);

        // Diagnostic stage: candidate generation only (no regex verification).
        group.bench_with_input(
            BenchmarkId::new("candidate_only", name),
            &(&file_ngrams, preplan.clone()),
            |b, (file_ngrams, plan)| {
                b.iter(|| {
                    let ids = candidate_ids_from_plan(black_box(plan), black_box(file_ngrams));
                    black_box(ids);
                });
            },
        );

        let pre_ids = candidate_ids_from_plan(&preplan, &file_ngrams);
        let pre_candidates = candidate_slices(&pre_ids, &files);

        // Diagnostic stage: verification only.
        group.bench_with_input(BenchmarkId::new("verify_only", name), pattern, |b, p| {
            b.iter(|| {
                let matches = verify_candidates(black_box(p), black_box(&pre_candidates));
                black_box(matches);
            });
        });

        // Diagnostic stage: search with precomputed plan (isolates planning overhead).
        group.bench_with_input(
            BenchmarkId::new("search_preplanned", name),
            &(&files, &file_ngrams, *pattern, preplan),
            |b, (files, file_ngrams, pattern, plan)| {
                b.iter(|| {
                    let results = search_pipeline_with_plan(
                        black_box(pattern),
                        black_box(files),
                        black_box(file_ngrams),
                        black_box(plan),
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
