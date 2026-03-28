//! Benchmark: n-gram extraction (trigram vs sparse)
//!
//! Measures build cost for trigram and sparse n-gram extraction.
//! Output: build throughput, grams produced, index-size estimate.

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use qndx_bench::fixtures;
use qndx_index::ngram::{extract_sparse_ngrams, extract_trigrams};

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

        // --- Sparse n-gram extraction over entire corpus ---
        group.bench_with_input(
            BenchmarkId::new("sparse/build", label),
            &corpus.files,
            |b, files| {
                b.iter(|| {
                    let mut total_grams = 0usize;
                    for f in files {
                        let grams = extract_sparse_ngrams(black_box(&f.content));
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
        let sparse_count: usize = corpus
            .files
            .iter()
            .map(|f| extract_sparse_ngrams(&f.content).len())
            .sum();
        eprintln!(
            "  [stats] {}: trigrams={}, sparse={}, ratio={:.2}x, corpus_bytes={}",
            label,
            trigram_count,
            sparse_count,
            sparse_count as f64 / trigram_count.max(1) as f64,
            total_bytes,
        );
    }

    group.finish();
}

criterion_group!(benches, bench_ngram_extract);
criterion_main!(benches);
