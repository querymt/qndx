//! Benchmark: postings representation choice (Decision Gate B)
//!
//! Compare Vec (varint-delta), Roaring, and hybrid posting list operations.
//! Workload: intersection/union over low/medium/high cardinality postings.
//! Output: query op latency + memory footprint + encode/decode throughput.
//!
//! Decision Gate B criteria:
//! - Choose hybrid when it is >=10% faster end-to-end query latency
//!   than single-format options and memory/index size does not exceed budget.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use qndx_index::postings::PostingList;
use rand::RngExt;
use rand::SeedableRng;
use rand::rngs::ChaCha8Rng;
use std::hint::black_box;

const SEED: u64 = 0xBEEF_CAFE_1234_5678;

/// Generate a sorted, deduplicated list of random file IDs.
fn random_ids(rng: &mut ChaCha8Rng, count: usize, max_id: u32) -> Vec<u32> {
    let mut ids: Vec<u32> = (0..count).map(|_| rng.random_range(0..max_id)).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn bench_postings_intersect_union(c: &mut Criterion) {
    let mut group = c.benchmark_group("postings_intersect_union");
    group.sample_size(100);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(3));

    let cardinalities = [
        ("low", 20, 1_000),
        ("medium", 500, 10_000),
        ("high", 5_000, 100_000),
    ];

    for (label, count, max_id) in &cardinalities {
        let mut rng = ChaCha8Rng::seed_from_u64(SEED);
        let ids_a = random_ids(&mut rng, *count, *max_id);
        let ids_b = random_ids(&mut rng, *count, *max_id);

        // --- Vec-only posting lists (forced) ---
        let vec_a = PostingList::force_vec(ids_a.clone());
        let vec_b = PostingList::force_vec(ids_b.clone());

        group.bench_with_input(
            BenchmarkId::new("vec/intersect", label),
            &(vec_a.clone(), vec_b.clone()),
            |b, (a, b_list)| {
                b.iter(|| black_box(black_box(a).intersect(black_box(b_list))));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("vec/union", label),
            &(vec_a.clone(), vec_b.clone()),
            |b, (a, b_list)| {
                b.iter(|| black_box(black_box(a).union(black_box(b_list))));
            },
        );

        // --- Roaring-only posting lists (forced) ---
        let roaring_a = PostingList::force_roaring(&ids_a);
        let roaring_b = PostingList::force_roaring(&ids_b);

        group.bench_with_input(
            BenchmarkId::new("roaring/intersect", label),
            &(roaring_a.clone(), roaring_b.clone()),
            |b, (a, b_list)| {
                b.iter(|| black_box(black_box(a).intersect(black_box(b_list))));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("roaring/union", label),
            &(roaring_a.clone(), roaring_b.clone()),
            |b, (a, b_list)| {
                b.iter(|| black_box(black_box(a).union(black_box(b_list))));
            },
        );

        // --- Hybrid posting lists (auto-selected by threshold) ---
        let hybrid_a = PostingList::from_vec(ids_a.clone());
        let hybrid_b = PostingList::from_vec(ids_b.clone());

        group.bench_with_input(
            BenchmarkId::new("hybrid/intersect", label),
            &(hybrid_a.clone(), hybrid_b.clone()),
            |b, (a, b_list)| {
                b.iter(|| black_box(black_box(a).intersect(black_box(b_list))));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("hybrid/union", label),
            &(hybrid_a.clone(), hybrid_b.clone()),
            |b, (a, b_list)| {
                b.iter(|| black_box(black_box(a).union(black_box(b_list))));
            },
        );
    }

    group.finish();
}

fn bench_postings_encode_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("postings_encode_decode");
    group.sample_size(100);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(3));

    let cardinalities = [
        ("low", 20, 1_000),
        ("medium", 500, 10_000),
        ("high", 5_000, 100_000),
    ];

    for (label, count, max_id) in &cardinalities {
        let mut rng = ChaCha8Rng::seed_from_u64(SEED);
        let ids = random_ids(&mut rng, *count, *max_id);

        // --- Encode benchmarks ---
        let vec_pl = PostingList::force_vec(ids.clone());
        let roaring_pl = PostingList::force_roaring(&ids);
        let hybrid_pl = PostingList::from_vec(ids.clone());

        // Vec fixed-width encode
        group.bench_with_input(
            BenchmarkId::new("vec_fixed/encode", label),
            &vec_pl,
            |b, pl| {
                b.iter(|| black_box(black_box(pl).encode_fixed()));
            },
        );

        // Vec varint encode
        group.bench_with_input(
            BenchmarkId::new("vec_varint/encode", label),
            &vec_pl,
            |b, pl| {
                b.iter(|| black_box(black_box(pl).encode_varint()));
            },
        );

        // Roaring encode
        group.bench_with_input(
            BenchmarkId::new("roaring/encode", label),
            &roaring_pl,
            |b, pl| {
                b.iter(|| black_box(black_box(pl).encode_roaring()));
            },
        );

        // Hybrid auto encode
        group.bench_with_input(
            BenchmarkId::new("hybrid_auto/encode", label),
            &hybrid_pl,
            |b, pl| {
                b.iter(|| black_box(black_box(pl).encode_auto()));
            },
        );

        // --- Decode benchmarks ---
        let fixed_bytes = vec_pl.encode_fixed();
        let varint_bytes = vec_pl.encode_varint();
        let roaring_bytes = roaring_pl.encode_roaring();
        let hybrid_bytes = hybrid_pl.encode_auto();

        group.bench_with_input(
            BenchmarkId::new("vec_fixed/decode", label),
            &fixed_bytes,
            |b, bytes| {
                b.iter(|| black_box(PostingList::decode_tagged(black_box(bytes))));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("vec_varint/decode", label),
            &varint_bytes,
            |b, bytes| {
                b.iter(|| black_box(PostingList::decode_tagged(black_box(bytes))));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("roaring/decode", label),
            &roaring_bytes,
            |b, bytes| {
                b.iter(|| black_box(PostingList::decode_tagged(black_box(bytes))));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("hybrid_auto/decode", label),
            &hybrid_bytes,
            |b, bytes| {
                b.iter(|| black_box(PostingList::decode_tagged(black_box(bytes))));
            },
        );

        // Print encoded sizes for reference
        if label == &"high" {
            eprintln!();
            eprintln!(
                "  === Encoded sizes for '{}' cardinality ({} ids) ===",
                label,
                ids.len()
            );
            eprintln!("    vec_fixed  : {:>8} bytes", fixed_bytes.len());
            eprintln!("    vec_varint : {:>8} bytes", varint_bytes.len());
            eprintln!("    roaring    : {:>8} bytes", roaring_bytes.len());
            eprintln!(
                "    hybrid_auto: {:>8} bytes ({})",
                hybrid_bytes.len(),
                if hybrid_pl.is_roaring() {
                    "roaring"
                } else {
                    "varint"
                },
            );
        }
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_postings_intersect_union,
    bench_postings_encode_decode
);
criterion_main!(benches);
