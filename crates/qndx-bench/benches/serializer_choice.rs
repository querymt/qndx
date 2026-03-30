//! Benchmark: serializer choice (Decision Gate A)
//!
//! Compare postcard vs wincode vs serde_json for manifest serialization.
//! Workload: manifest encode/decode at realistic sizes and frequencies.
//! Output: latency, throughput, encoded size.
//!
//! Decision Gate A criteria:
//! - If wincode provides >=15% better decode throughput on manifest-heavy workloads
//!   AND no compatibility/maintenance concerns arise, choose wincode.
//! - Otherwise choose postcard for simplicity and stable wire format.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

fn bench_serializer_choice(c: &mut Criterion) {
    let manifests = qndx_bench::fixtures::sample_manifests();

    let mut group = c.benchmark_group("serializer_choice");

    // Lock benchmark environment knobs
    group.sample_size(100);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(3));

    for (label, manifest) in &manifests {
        // --- postcard encode ---
        let postcard_bytes = postcard::to_allocvec(manifest).unwrap();
        group.throughput(Throughput::Bytes(postcard_bytes.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("postcard/encode", label),
            manifest,
            |b, m| {
                b.iter(|| {
                    let encoded = postcard::to_allocvec(black_box(m)).unwrap();
                    black_box(encoded);
                });
            },
        );

        // --- postcard decode ---
        group.bench_with_input(
            BenchmarkId::new("postcard/decode", label),
            &postcard_bytes,
            |b, bytes| {
                b.iter(|| {
                    let decoded: qndx_core::Manifest =
                        postcard::from_bytes(black_box(bytes)).unwrap();
                    black_box(decoded);
                });
            },
        );

        // --- wincode encode ---
        let wincode_bytes = wincode::serialize(manifest).unwrap();
        group.bench_with_input(
            BenchmarkId::new("wincode/encode", label),
            manifest,
            |b, m| {
                b.iter(|| {
                    let encoded = wincode::serialize(black_box(m)).unwrap();
                    black_box(encoded);
                });
            },
        );

        // --- wincode decode ---
        group.bench_with_input(
            BenchmarkId::new("wincode/decode", label),
            &wincode_bytes,
            |b, bytes| {
                b.iter(|| {
                    let decoded: qndx_core::Manifest =
                        wincode::deserialize(black_box(bytes)).unwrap();
                    black_box(decoded);
                });
            },
        );

        // --- serde_json encode (baseline comparison) ---
        let json_bytes = serde_json::to_vec(manifest).unwrap();
        group.bench_with_input(
            BenchmarkId::new("serde_json/encode", label),
            manifest,
            |b, m| {
                b.iter(|| {
                    let encoded = serde_json::to_vec(black_box(m)).unwrap();
                    black_box(encoded);
                });
            },
        );

        // --- serde_json decode ---
        group.bench_with_input(
            BenchmarkId::new("serde_json/decode", label),
            &json_bytes,
            |b, bytes| {
                b.iter(|| {
                    let decoded: qndx_core::Manifest =
                        serde_json::from_slice(black_box(bytes)).unwrap();
                    black_box(decoded);
                });
            },
        );

        // Print encoded sizes for reference (once, outside timing)
        if label == "large" {
            eprintln!();
            eprintln!(
                "  === Encoded sizes for '{}' manifest ({} files) ===",
                label, manifest.file_count
            );
            eprintln!("    postcard : {:>8} bytes", postcard_bytes.len());
            eprintln!("    wincode  : {:>8} bytes", wincode_bytes.len());
            eprintln!("    json     : {:>8} bytes", json_bytes.len());
            eprintln!(
                "    ratio: json/postcard={:.2}x, wincode/postcard={:.2}x",
                json_bytes.len() as f64 / postcard_bytes.len() as f64,
                wincode_bytes.len() as f64 / postcard_bytes.len() as f64,
            );
        }
    }

    group.finish();
}

criterion_group!(benches, bench_serializer_choice);
criterion_main!(benches);
