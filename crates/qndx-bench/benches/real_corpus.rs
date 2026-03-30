//! Benchmark: real codebase end-to-end performance
//!
//! Benchmarks qndx against a real codebase specified via environment variables.
//! Skips gracefully when QNDX_BENCH_CORPUS is not set.
//!
//! Environment variables:
//!   QNDX_BENCH_CORPUS       - Path to the codebase to benchmark (required)
//!   QNDX_BENCH_PATTERNS     - Path to a patterns file (optional, one per line)
//!   QNDX_BENCH_NAME         - Override corpus name in reports (optional)
//!   QNDX_BENCH_MAX_FILES    - Limit number of files (optional)
//!   QNDX_BENCH_MAX_FILE_SIZE - Max file size in bytes (optional, default 1MB)
//!
//! Usage:
//!   QNDX_BENCH_CORPUS=~/src/linux cargo bench --bench real_corpus
//!   QNDX_BENCH_CORPUS=~/qmt/querymt QNDX_BENCH_PATTERNS=benchmarks/patterns/rust.txt cargo bench --bench real_corpus

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use qndx_bench::fixtures::{
    self, ExternalCorpusConfig, NamedPattern, corpus_bench_name, human_bytes,
};
use qndx_core::scan;
use qndx_core::walk::WalkConfig;
use qndx_index::IndexReader;
use qndx_query::planner::StrategyOverride;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Maximum file count before we skip the index build benchmark.
/// Building a 1GB+ corpus in a Criterion loop is not useful.
const BUILD_BENCH_FILE_LIMIT: usize = 10_000;

/// Precomputed state shared across all benchmarks in a run.
struct BenchState {
    /// Short name for benchmark group naming.
    corpus_name: String,
    /// Root path of the corpus.
    root: PathBuf,
    /// Number of files in the corpus.
    file_count: usize,
    /// Total bytes across all files.
    total_bytes: u64,
    /// Temp directory handle (keeps index files alive until dropped).
    _temp_dir: tempfile::TempDir,
    /// Pre-opened index reader (memory-mapped).
    reader: IndexReader,
    /// Patterns to benchmark.
    patterns: Vec<NamedPattern>,
}

impl BenchState {
    fn build(root: &Path) -> Self {
        let config = ExternalCorpusConfig::from_env();
        let corpus_name = corpus_bench_name(root);

        eprintln!();
        eprintln!("=== Real Corpus Benchmark Setup ===");
        eprintln!("Corpus:     {}", root.display());
        eprintln!("Name:       {}", corpus_name);

        let walk_config = WalkConfig {
            max_file_size: config.max_file_size,
            ..Default::default()
        };

        // Count files and total size without loading content into memory
        let discovered = qndx_core::walk::discover_files(root, &walk_config);
        let limit = config.max_files.unwrap_or(usize::MAX);
        let file_count = discovered.len().min(limit);

        let total_bytes: u64 = discovered
            .iter()
            .take(limit)
            .filter_map(|f| std::fs::metadata(&f.abs_path).ok())
            .map(|m| m.len())
            .sum();

        eprintln!("Files:      {}", file_count);
        eprintln!("Total size: {}", human_bytes(total_bytes));

        // Build index using streaming (reads files one at a time)
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir for index");
        let index_dir = temp_dir.path().join("index/v1");

        eprintln!("Building index (streaming)...");
        let build_start = Instant::now();

        // Use the streaming build_index_from_dir which reads files one at a time
        let build_config = WalkConfig {
            max_file_size: config.max_file_size,
            ..Default::default()
        };
        let build_result = qndx_index::build_index_from_dir(root, &index_dir, &build_config, None)
            .expect("failed to build index");
        let build_elapsed = build_start.elapsed();

        let index_size: u64 = std::fs::read_dir(&index_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();

        eprintln!(
            "Index:      {} ({:.1}% of corpus) in {:.3}s",
            human_bytes(index_size),
            index_size as f64 / total_bytes.max(1) as f64 * 100.0,
            build_elapsed.as_secs_f64(),
        );
        eprintln!(
            "N-grams:    {} ({} trigram, {} sparse)",
            build_result.ngram_count, build_result.trigram_count, build_result.sparse_count,
        );
        eprintln!(
            "Throughput: {:.1} MB/s",
            total_bytes as f64 / 1_048_576.0 / build_elapsed.as_secs_f64(),
        );

        // Open with mmap (near-instant, no heap allocation for index data)
        let reader = IndexReader::open(&index_dir).expect("failed to open index");
        let patterns = fixtures::real_corpus_patterns();

        eprintln!("Patterns:   {}", patterns.len());
        eprintln!("===================================");
        eprintln!();

        BenchState {
            corpus_name,
            root: root.to_path_buf(),
            file_count,
            total_bytes,
            _temp_dir: temp_dir,
            reader,
            patterns,
        }
    }
}

// ---------------------------------------------------------------------------
// Index build benchmark (skipped for large corpora)
// ---------------------------------------------------------------------------

fn bench_index_build(c: &mut Criterion, state: &BenchState) {
    if state.file_count > BUILD_BENCH_FILE_LIMIT {
        eprintln!(
            "Skipping build benchmark: {} files exceeds limit of {}. \
             Set QNDX_BENCH_BUILD=1 to force.",
            state.file_count, BUILD_BENCH_FILE_LIMIT,
        );
        if std::env::var("QNDX_BENCH_BUILD").unwrap_or_default() != "1" {
            return;
        }
    }

    let group_name = format!("real_{}/build", state.corpus_name);
    let mut group = c.benchmark_group(&group_name);
    group.sample_size(10);
    group.warm_up_time(std::time::Duration::from_secs(2));
    group.measurement_time(std::time::Duration::from_secs(10));
    group.throughput(Throughput::Bytes(state.total_bytes));

    let walk_config = WalkConfig::default();
    let root = state.root.clone();

    group.bench_function("index_build", |b| {
        b.iter(|| {
            let temp = tempfile::tempdir().unwrap();
            let idx_dir = temp.path().join("index/v1");
            let result =
                qndx_index::build_index_from_dir(black_box(&root), &idx_dir, &walk_config, None);
            black_box(result).unwrap();
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Indexed search benchmarks (auto / trigram / sparse)
// ---------------------------------------------------------------------------

fn bench_search_indexed(c: &mut Criterion, state: &BenchState) {
    let strategies: &[(&str, StrategyOverride)] = &[
        ("auto", StrategyOverride::Auto),
        ("trigram", StrategyOverride::ForceTrigram),
        ("sparse", StrategyOverride::ForceSparse),
    ];

    for (strategy_name, strategy) in strategies {
        let group_name = format!("real_{}/indexed/{}", state.corpus_name, strategy_name);
        let mut group = c.benchmark_group(&group_name);
        group.sample_size(20);
        group.warm_up_time(std::time::Duration::from_secs(1));
        group.measurement_time(std::time::Duration::from_secs(5));

        for pat in &state.patterns {
            group.bench_with_input(
                BenchmarkId::from_parameter(&pat.name),
                &pat.pattern,
                |b, pattern| {
                    b.iter(|| {
                        let result = qndx_query::index_search_with_strategy_and_timing(
                            &state.reader,
                            black_box(&state.root),
                            black_box(pattern),
                            *strategy,
                            false,
                        );
                        black_box(result).ok()
                    });
                },
            );
        }

        group.finish();
    }
}

// ---------------------------------------------------------------------------
// Indexed search benchmarks (timing off vs on overhead)
// ---------------------------------------------------------------------------

fn bench_search_timing_overhead(c: &mut Criterion, state: &BenchState) {
    let group_name = format!("real_{}/indexed/timing_overhead", state.corpus_name);
    let mut group = c.benchmark_group(&group_name);
    group.sample_size(20);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(5));

    for pat in &state.patterns {
        group.bench_with_input(
            BenchmarkId::new("timing_off", &pat.name),
            &pat.pattern,
            |b, pattern| {
                b.iter(|| {
                    let result = qndx_query::index_search_with_strategy_and_timing(
                        &state.reader,
                        black_box(&state.root),
                        black_box(pattern),
                        StrategyOverride::Auto,
                        false,
                    );
                    black_box(result).ok()
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("timing_on", &pat.name),
            &pat.pattern,
            |b, pattern| {
                b.iter(|| {
                    let result = qndx_query::index_search_with_strategy_and_timing(
                        &state.reader,
                        black_box(&state.root),
                        black_box(pattern),
                        StrategyOverride::Auto,
                        true,
                    );
                    black_box(result).ok()
                });
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Index open benchmark (mmap + header/checksum validation)
// ---------------------------------------------------------------------------

fn bench_index_open(c: &mut Criterion, state: &BenchState) {
    let group_name = format!("real_{}/index_open", state.corpus_name);
    let mut group = c.benchmark_group(&group_name);
    group.sample_size(10);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(5));

    let index_dir = state._temp_dir.path().join("index/v1");
    group.bench_function("open", |b| {
        b.iter(|| {
            let reader = IndexReader::open(black_box(&index_dir));
            black_box(reader).unwrap();
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Scan search benchmark (baseline comparison)
// ---------------------------------------------------------------------------

fn bench_search_scan(c: &mut Criterion, state: &BenchState) {
    let group_name = format!("real_{}/scan", state.corpus_name);
    let mut group = c.benchmark_group(&group_name);
    group.sample_size(10);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(10));

    let walk_config = WalkConfig::default();

    for pat in &state.patterns {
        group.bench_with_input(
            BenchmarkId::from_parameter(&pat.name),
            &pat.pattern,
            |b, pattern| {
                b.iter(|| {
                    let result =
                        scan::scan_search(black_box(&state.root), black_box(pattern), &walk_config);
                    black_box(result).ok()
                });
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Summary table (printed after all benchmarks)
// ---------------------------------------------------------------------------

fn print_summary(state: &BenchState) {
    eprintln!();
    eprintln!("=== Real Corpus Benchmark Summary ===");
    eprintln!(
        "Corpus: {} ({} files, {})",
        state.root.display(),
        state.file_count,
        human_bytes(state.total_bytes),
    );
    eprintln!();
    eprintln!(
        "  {:<30} {:>8} {:>10} {:>8} {:>10} {:>10} {:>8}",
        "pattern", "strategy", "candidates", "matches", "scan(ms)", "index(ms)", "speedup",
    );
    eprintln!("  {}", "-".repeat(88));

    let walk_config = WalkConfig::default();

    for pat in &state.patterns {
        // Run scan once for timing
        let scan_start = Instant::now();
        let scan_result = scan::scan_search(&state.root, &pat.pattern, &walk_config);
        let scan_ms = scan_start.elapsed().as_secs_f64() * 1000.0;
        let scan_matches = scan_result.as_ref().map(|r| r.matches.len()).unwrap_or(0);

        // Run indexed search once for timing and stats
        let idx_start = Instant::now();
        let idx_result = qndx_query::index_search_with_strategy(
            &state.reader,
            &state.root,
            &pat.pattern,
            StrategyOverride::Auto,
        );
        let idx_ms = idx_start.elapsed().as_secs_f64() * 1000.0;

        let (strategy, candidates, idx_matches) = match &idx_result {
            Ok(r) => (
                format!("{}", r.stats.strategy),
                r.stats.candidate_count,
                r.results.matches.len(),
            ),
            Err(_) => ("error".into(), 0, 0),
        };

        let speedup = if idx_ms > 0.0 && scan_ms > 0.0 {
            format!("{:.1}x", scan_ms / idx_ms)
        } else {
            "N/A".into()
        };

        // Truncate pattern name for display
        let display_name = if pat.name.len() > 30 {
            format!("{}...", &pat.name[..27])
        } else {
            pat.name.clone()
        };

        eprintln!(
            "  {:<30} {:>8} {:>10} {:>8} {:>10.1} {:>10.1} {:>8}",
            display_name, strategy, candidates, idx_matches, scan_ms, idx_ms, speedup,
        );

        // Flag mismatches
        if scan_matches != idx_matches {
            eprintln!(
                "  ** MISMATCH: scan found {} matches, index found {} **",
                scan_matches, idx_matches,
            );
        }
    }

    eprintln!("  {}", "-".repeat(88));
    eprintln!();
}

// ---------------------------------------------------------------------------
// Main benchmark entry point
// ---------------------------------------------------------------------------

fn bench_real_corpus(c: &mut Criterion) {
    let corpus_path = match std::env::var("QNDX_BENCH_CORPUS") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            eprintln!();
            eprintln!("Skipping real_corpus benchmarks: QNDX_BENCH_CORPUS not set.");
            eprintln!("Usage: QNDX_BENCH_CORPUS=/path/to/repo cargo bench --bench real_corpus");
            eprintln!();
            return;
        }
    };

    if !corpus_path.is_dir() {
        eprintln!(
            "error: QNDX_BENCH_CORPUS={} is not a directory",
            corpus_path.display()
        );
        return;
    }

    let state = BenchState::build(&corpus_path);

    if state.file_count == 0 {
        eprintln!("error: no files found in corpus {}", corpus_path.display());
        return;
    }

    // Run benchmark groups
    bench_index_build(c, &state);
    bench_index_open(c, &state);
    bench_search_indexed(c, &state);
    bench_search_timing_overhead(c, &state);
    bench_search_scan(c, &state);

    // Print summary table
    print_summary(&state);
}

criterion_group!(benches, bench_real_corpus);
criterion_main!(benches);
