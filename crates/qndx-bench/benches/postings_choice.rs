//! Benchmark: postings representation choice (Decision Gate B)
//!
//! Compare Vec<u32>, Roaring, and hybrid posting list operations.
//! Workload: intersection/union over low/medium/high cardinality postings.
//! Output: query op latency + memory footprint.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use qndx_index::postings::PostingList;
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

const SEED: u64 = 0xBEEF_CAFE_1234_5678;

/// Generate a sorted, deduplicated list of random file IDs.
fn random_ids(rng: &mut ChaCha8Rng, count: usize, max_id: u32) -> Vec<u32> {
    let mut ids: Vec<u32> = (0..count).map(|_| rng.gen_range(0..max_id)).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn bench_postings_choice(c: &mut Criterion) {
    let mut group = c.benchmark_group("postings_choice");
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

        // --- Vec-only posting lists ---
        let vec_a = PostingList::from_vec(ids_a.clone());
        let vec_b = PostingList::from_vec(ids_b.clone());

        group.bench_with_input(
            BenchmarkId::new("intersect", label),
            &(vec_a.clone(), vec_b.clone()),
            |b, (a, b_list)| {
                b.iter(|| {
                    let result = black_box(a).intersect(black_box(b_list));
                    black_box(result);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("union", label),
            &(vec_a.clone(), vec_b.clone()),
            |b, (a, b_list)| {
                b.iter(|| {
                    let result = black_box(a).union(black_box(b_list));
                    black_box(result);
                });
            },
        );

        // --- Force Roaring for comparison on high cardinality ---
        if *count >= 500 {
            let roaring_a = force_roaring(&ids_a);
            let roaring_b = force_roaring(&ids_b);

            group.bench_with_input(
                BenchmarkId::new("intersect_roaring", label),
                &(roaring_a.clone(), roaring_b.clone()),
                |b, (a, b_list)| {
                    b.iter(|| {
                        let result = black_box(a).intersect(black_box(b_list));
                        black_box(result);
                    });
                },
            );

            group.bench_with_input(
                BenchmarkId::new("union_roaring", label),
                &(roaring_a.clone(), roaring_b.clone()),
                |b, (a, b_list)| {
                    b.iter(|| {
                        let result = black_box(a).union(black_box(b_list));
                        black_box(result);
                    });
                },
            );
        }
    }

    group.finish();
}

/// Force a posting list into Roaring representation regardless of size.
fn force_roaring(ids: &[u32]) -> PostingList {
    let mut bitmap = roaring::RoaringBitmap::new();
    for &id in ids {
        bitmap.insert(id);
    }
    PostingList::Roaring(bitmap)
}

criterion_group!(benches, bench_postings_choice);
criterion_main!(benches);
