//! Benchmark: n-gram extraction (trigram vs sparse)
//!
//! Measures build cost for trigram and sparse n-gram extraction.
//! Output: build throughput, grams produced, index-size estimate.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use qndx_bench::fixtures;
use qndx_index::ngram::{
    extract_sparse_ngrams_all, extract_sparse_ngrams_covering, extract_trigrams,
};
use std::hint::black_box;

fn bench_ngram_extract(c: &mut Criterion) {
    let mut group = c.benchmark_group("ngram_extract");
    group.sample_size(50);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(5));

    let corpora = [
        ("small", fixtures::small_corpus()),
        ("medium", fixtures::medium_corpus()),
    ];

    for (label, corpus) in &corpora {
        let total_bytes: usize = corpus.files.iter().map(|f| f.content.len()).sum();
        group.throughput(Throughput::Bytes(total_bytes as u64));

        // --- Trigram extraction over entire corpus ---
        group.bench_with_input(
            BenchmarkId::new("trigram/build", label),
            &corpus.files,
            |b, files| {
                b.iter(|| {
                    let mut total_grams = 0usize;
                    for f in files {
                        let grams = extract_trigrams(black_box(&f.content));
                        total_grams += grams.len();
                    }
                    black_box(total_grams)
                });
            },
        );

        // --- Sparse n-gram build_all extraction (index time) ---
        group.bench_with_input(
            BenchmarkId::new("sparse_all/build", label),
            &corpus.files,
            |b, files| {
                b.iter(|| {
                    let mut total_grams = 0usize;
                    for f in files {
                        let grams = extract_sparse_ngrams_all(black_box(&f.content));
                        total_grams += grams.len();
                    }
                    black_box(total_grams)
                });
            },
        );

        // --- Sparse n-gram build_covering extraction (query time) ---
        group.bench_with_input(
            BenchmarkId::new("sparse_covering/build", label),
            &corpus.files,
            |b, files| {
                b.iter(|| {
                    let mut total_grams = 0usize;
                    for f in files {
                        let grams = extract_sparse_ngrams_covering(black_box(&f.content));
                        total_grams += grams.len();
                    }
                    black_box(total_grams)
                });
            },
        );

        // Print gram counts for reference (outside timed section)
        let trigram_count: usize = corpus
            .files
            .iter()
            .map(|f| extract_trigrams(&f.content).len())
            .sum();
        let sparse_all_count: usize = corpus
            .files
            .iter()
            .map(|f| extract_sparse_ngrams_all(&f.content).len())
            .sum();
        let sparse_cov_count: usize = corpus
            .files
            .iter()
            .map(|f| extract_sparse_ngrams_covering(&f.content).len())
            .sum();
        eprintln!(
            "  [stats] {}: trigrams={}, sparse_all={}, sparse_covering={}, corpus_bytes={}",
            label, trigram_count, sparse_all_count, sparse_cov_count, total_bytes,
        );
    }

    group.finish();
}

criterion_group!(benches, bench_ngram_extract);
criterion_main!(benches);
