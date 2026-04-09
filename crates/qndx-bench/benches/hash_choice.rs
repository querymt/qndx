//! Benchmark: hash function choice (Decision Gate D)
//!
//! Compares crc32fast, rapidhash, ahash, and xxhash-rust across:
//! - Micro workloads (2-20 byte n-gram hashing)
//! - Pair-weight hashing (2-byte hot path)
//! - Bulk checksums (1KB-16MB)
//! - End-to-end extraction pipeline throughput

use ahash::AHasher;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use crc32fast::Hasher as Crc32Hasher;
use qndx_bench::fixtures;
use std::hash::Hasher;
use std::hint::black_box;
use xxhash_rust::xxh3::xxh3_64;

#[derive(Clone, Copy)]
enum HashImpl {
    Crc32,
    RapidHash,
    AHash,
    Xxh3,
}

impl HashImpl {
    fn name(self) -> &'static str {
        match self {
            HashImpl::Crc32 => "crc32",
            HashImpl::RapidHash => "rapidhash",
            HashImpl::AHash => "ahash",
            HashImpl::Xxh3 => "xxh3",
        }
    }

    #[inline]
    fn hash32(self, bytes: &[u8]) -> u32 {
        match self {
            HashImpl::Crc32 => {
                let mut hasher = Crc32Hasher::new();
                hasher.update(bytes);
                hasher.finalize()
            }
            HashImpl::RapidHash => rapidhash::v3::rapidhash_v3(bytes) as u32,
            HashImpl::AHash => {
                let mut hasher = AHasher::default();
                hasher.write(bytes);
                hasher.finish() as u32
            }
            HashImpl::Xxh3 => xxh3_64(bytes) as u32,
        }
    }

    #[inline]
    fn hash64(self, bytes: &[u8]) -> u64 {
        match self {
            HashImpl::Crc32 => self.hash32(bytes) as u64,
            HashImpl::RapidHash => rapidhash::v3::rapidhash_v3(bytes),
            HashImpl::AHash => {
                let mut hasher = AHasher::default();
                hasher.write(bytes);
                hasher.finish()
            }
            HashImpl::Xxh3 => xxh3_64(bytes),
        }
    }
}

fn hash_candidates() -> [HashImpl; 4] {
    [
        HashImpl::Crc32,
        HashImpl::RapidHash,
        HashImpl::AHash,
        HashImpl::Xxh3,
    ]
}

fn bench_ngram_hash_micro(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_choice/ngram_hash");
    group.sample_size(120);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(4));

    let lengths = [2usize, 3, 5, 8, 12, 20];

    for &len in &lengths {
        let mut input = vec![0u8; len];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(17).wrapping_add(31);
        }

        group.throughput(Throughput::Elements(1));

        for hash_impl in hash_candidates() {
            group.bench_with_input(
                BenchmarkId::new(hash_impl.name(), format!("len_{len}")),
                &input,
                |b, data| {
                    b.iter(|| {
                        let h = hash_impl.hash32(black_box(data));
                        black_box(h)
                    })
                },
            );
        }
    }

    group.finish();
}

fn bench_pair_weight_micro(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_choice/pair_weight");
    group.sample_size(150);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(4));

    let pairs: Vec<[u8; 2]> = (0u8..=255)
        .step_by(17)
        .flat_map(|a| (0u8..=255).step_by(29).map(move |b| [a, b]))
        .collect();

    group.throughput(Throughput::Elements(pairs.len() as u64));

    for hash_impl in hash_candidates() {
        group.bench_with_input(BenchmarkId::new(hash_impl.name(), "batch"), &pairs, |b, ps| {
            b.iter(|| {
                let mut acc = 0u32;
                for p in ps {
                    acc ^= hash_impl.hash32(black_box(p));
                }
                black_box(acc)
            })
        });
    }

    group.finish();
}

fn bench_bulk_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_choice/bulk_checksum");
    group.sample_size(60);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(5));

    let sizes = [1024usize, 64 * 1024, 1024 * 1024, 16 * 1024 * 1024];

    for &size in &sizes {
        let data: Vec<u8> = (0..size)
            .map(|i| (i as u8).wrapping_mul(13).wrapping_add(7))
            .collect();
        group.throughput(Throughput::Bytes(size as u64));

        for hash_impl in hash_candidates() {
            group.bench_with_input(
                BenchmarkId::new(hash_impl.name(), format!("{}kb", size / 1024)),
                &data,
                |b, bytes| {
                    b.iter(|| {
                        let h = hash_impl.hash64(black_box(bytes));
                        black_box(h)
                    })
                },
            );
        }
    }

    group.finish();
}

#[inline]
fn extract_trigrams_with_hash(data: &[u8], hash_impl: HashImpl) -> Vec<u32> {
    if data.len() < 3 {
        return Vec::new();
    }
    let mut hashes: Vec<u32> = data.windows(3).map(|w| hash_impl.hash32(w)).collect();
    hashes.sort_unstable();
    hashes.dedup();
    hashes
}

#[inline]
fn extract_sparse_ngrams_all_with_hash(data: &[u8], hash_impl: HashImpl) -> Vec<(u32, usize)> {
    if data.len() < 2 {
        return Vec::new();
    }

    let weights: Vec<u32> = data.windows(2).map(|w| hash_impl.hash32(w)).collect();
    let n = weights.len();
    let mut ngrams = Vec::new();

    for i in 0..n {
        let gram = &data[i..i + 2];
        ngrams.push((hash_impl.hash32(gram), gram.len()));

        let mut interior_max: u32 = 0;
        for j in (i + 1)..n {
            if j > i + 1 {
                interior_max = interior_max.max(weights[j - 1]);
            }

            if interior_max >= weights[i] {
                break;
            }

            if weights[j] > interior_max {
                let end = j + 2;
                if end <= data.len() {
                    let gram = &data[i..end];
                    ngrams.push((hash_impl.hash32(gram), gram.len()));
                }
            }
        }
    }

    ngrams.sort_unstable();
    ngrams.dedup();
    ngrams
}

fn bench_extract_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_choice/extract_pipeline");
    group.sample_size(20);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(6));

    let corpora = [
        ("small", fixtures::small_corpus()),
        ("medium", fixtures::medium_corpus()),
    ];

    for (label, corpus) in corpora {
        let total_bytes = corpus.total_bytes() as u64;
        group.throughput(Throughput::Bytes(total_bytes));

        for hash_impl in hash_candidates() {
            group.bench_with_input(
                BenchmarkId::new(format!("{}/trigram", hash_impl.name()), label),
                &corpus.files,
                |b, files| {
                    b.iter(|| {
                        let mut total = 0usize;
                        for f in files {
                            total +=
                                extract_trigrams_with_hash(black_box(&f.content), hash_impl).len();
                        }
                        black_box(total)
                    })
                },
            );

            group.bench_with_input(
                BenchmarkId::new(format!("{}/sparse_all", hash_impl.name()), label),
                &corpus.files,
                |b, files| {
                    b.iter(|| {
                        let mut total = 0usize;
                        for f in files {
                            total += extract_sparse_ngrams_all_with_hash(
                                black_box(&f.content),
                                hash_impl,
                            )
                            .len();
                        }
                        black_box(total)
                    })
                },
            );
        }
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_ngram_hash_micro,
    bench_pair_weight_micro,
    bench_bulk_hash,
    bench_extract_pipeline
);
criterion_main!(benches);
