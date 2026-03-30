//! qndx CLI: index, search, bench, and report commands.

use clap::{Parser, Subcommand, ValueEnum};
use qndx_core::scan;
use qndx_core::walk::WalkConfig;
use qndx_query::StrategyOverride;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Default index directory relative to the repository root.
const DEFAULT_INDEX_DIR: &str = ".qndx/index/v1";

#[derive(Parser)]
#[command(
    name = "qndx",
    about = "Fast regex search indexer for large repositories"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build the search index for a directory
    Index {
        /// Root directory to index (defaults to current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,
        /// Index output directory
        #[arg(short, long)]
        index_dir: Option<PathBuf>,
        /// Maximum file size in bytes
        #[arg(long, default_value = "1048576")]
        max_file_size: u64,
        /// Include hidden files
        #[arg(long)]
        hidden: bool,
        /// Include binary files
        #[arg(long)]
        binary: bool,
    },
    /// Search using regex
    Search {
        /// Regex pattern to search for
        pattern: String,
        /// Root directory to search (defaults to current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,
        /// Index directory (if present, use index-backed search)
        #[arg(short, long)]
        index_dir: Option<PathBuf>,
        /// Maximum file size in bytes
        #[arg(long, default_value = "1048576")]
        max_file_size: u64,
        /// Include hidden files
        #[arg(long)]
        hidden: bool,
        /// Include binary files
        #[arg(long)]
        binary: bool,
        /// Show only file names (no match details)
        #[arg(short = 'l', long)]
        files_only: bool,
        /// Show timing statistics
        #[arg(long)]
        stats: bool,
        /// Force scan-only mode (ignore index even if present)
        #[arg(long)]
        scan: bool,
        /// N-gram strategy: auto (default), trigram, or sparse
        #[arg(long, default_value = "auto")]
        strategy: StrategyArg,
    },
    /// Show query plan diagnostics (decomposition, costs, strategy selection)
    Plan {
        /// Regex pattern to analyze
        pattern: String,
        /// Force a specific strategy for the plan
        #[arg(long, default_value = "auto")]
        strategy: StrategyArg,
    },
    /// Benchmark operations
    Bench {
        #[command(subcommand)]
        action: BenchAction,
    },
}

/// N-gram strategy selection for the CLI.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum StrategyArg {
    /// Let the planner pick the best strategy
    Auto,
    /// Force trigram decomposition
    Trigram,
    /// Force sparse n-gram covering
    Sparse,
}

impl From<StrategyArg> for StrategyOverride {
    fn from(arg: StrategyArg) -> Self {
        match arg {
            StrategyArg::Auto => StrategyOverride::Auto,
            StrategyArg::Trigram => StrategyOverride::ForceTrigram,
            StrategyArg::Sparse => StrategyOverride::ForceSparse,
        }
    }
}

#[derive(Subcommand)]
enum BenchAction {
    /// Generate a benchmark report
    Report {
        /// Output format: "human" or "json"
        #[arg(long, default_value = "human")]
        format: String,
        /// Path to Criterion target directory
        #[arg(long, default_value = "target/criterion")]
        criterion_dir: String,
    },
    /// Check performance budgets against baseline
    CheckBudgets {
        /// Path to benchmark comparison output (JSON)
        #[arg(short, long)]
        comparison: Option<PathBuf>,
        /// Path to budgets configuration file
        #[arg(long, default_value = "benchmarks/budgets.toml")]
        budgets: PathBuf,
        /// Fail on critical budget violations
        #[arg(long, default_value = "true")]
        fail_on_critical: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Index {
            root,
            index_dir,
            max_file_size,
            hidden,
            binary,
        } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            let index_dir = index_dir.unwrap_or_else(|| root.join(DEFAULT_INDEX_DIR));
            let config = WalkConfig {
                max_file_size,
                include_hidden: hidden,
                skip_binary: !binary,
                ..Default::default()
            };

            let start = Instant::now();
            eprintln!("Building index for {} ...", root.display());

            match qndx_index::build_index_from_dir(&root, &index_dir, &config, None) {
                Ok(result) => {
                    let elapsed = start.elapsed();
                    eprintln!(
                        "Indexed {} files ({} trigrams, {} bytes postings) from {} source bytes in {:.3}s",
                        result.file_count,
                        result.ngram_count,
                        result.postings_bytes,
                        result.source_bytes,
                        elapsed.as_secs_f64(),
                    );
                    eprintln!("Index written to {}", index_dir.display());
                }
                Err(e) => {
                    eprintln!("error building index: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Search {
            pattern,
            root,
            index_dir,
            max_file_size,
            hidden,
            binary,
            files_only,
            stats,
            scan: force_scan,
            strategy,
        } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            let config = WalkConfig {
                max_file_size,
                include_hidden: hidden,
                skip_binary: !binary,
                ..Default::default()
            };
            let strategy_override: StrategyOverride = strategy.into();

            // Determine if we should use index-backed search
            let effective_index_dir = if force_scan {
                None
            } else {
                let dir = index_dir.unwrap_or_else(|| root.join(DEFAULT_INDEX_DIR));
                if dir.join("ngrams.tbl").exists() {
                    Some(dir)
                } else {
                    None
                }
            };

            let start = Instant::now();

            if let Some(idx_dir) = effective_index_dir {
                // Index-backed search
                run_index_search(
                    &root,
                    &idx_dir,
                    &pattern,
                    files_only,
                    stats,
                    start,
                    strategy_override,
                );
            } else {
                // Scan-only search
                run_scan_search(&root, &pattern, &config, files_only, stats, start);
            }
        }
        Commands::Plan { pattern, strategy } => {
            let strategy_override: StrategyOverride = strategy.into();
            let diag = qndx_query::plan_diagnostics_with_strategy(&pattern, strategy_override);

            println!("Pattern: {}", pattern);
            println!();

            if diag.literals.is_empty() {
                println!("Literals: (none extracted)");
            } else {
                println!("Literals: {:?}", diag.literals);
            }
            println!();

            println!("Trigram plan:");
            println!("  lookups: {}", diag.trigram_lookups);
            println!("  cost:    {:.2}", diag.trigram_cost);
            println!("  hashes:  {:?}", diag.selected.decomposition.required);
            println!();

            let sparse_gram_count = diag.selected.decomposition.sparse_required.len();
            let sparse_grams_display: Vec<String> = diag
                .selected
                .decomposition
                .sparse_required
                .iter()
                .map(|g| format!("hash={:#010x} len={}", g.hash, g.gram_len))
                .collect();

            match (diag.sparse_lookups, diag.sparse_cost) {
                (Some(lookups), Some(cost)) => {
                    println!("Sparse plan:");
                    println!("  lookups: {}", lookups);
                    println!("  cost:    {:.2}", cost);
                    println!("  grams:   {:?}", sparse_grams_display);
                }
                _ => {
                    println!(
                        "Sparse plan: unavailable ({} sparse grams >= {} trigrams, no reduction)",
                        sparse_gram_count, diag.trigram_lookups,
                    );
                    if !sparse_grams_display.is_empty() {
                        println!("  grams:   {:?}", sparse_grams_display);
                    }
                }
            }
            println!();

            println!("Selected:  {}", diag.selected.strategy);
            println!("Lookups:   {}", diag.selected.lookup_count);
            println!("Cost:      {:.2}", diag.selected.estimated_cost);
        }
        Commands::Bench { action } => match action {
            BenchAction::Report {
                format,
                criterion_dir,
            } => {
                qndx_bench::report::generate_report(&criterion_dir, &format);
            }
            BenchAction::CheckBudgets {
                comparison,
                budgets,
                fail_on_critical,
            } => {
                match qndx_bench::report::check_performance_budgets(
                    comparison.as_deref(),
                    &budgets,
                    fail_on_critical,
                ) {
                    Ok(passed) => {
                        if !passed {
                            std::process::exit(1);
                        }
                    }
                    Err(e) => {
                        eprintln!("error checking budgets: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        },
    }
}

fn run_index_search(
    root: &Path,
    index_dir: &Path,
    pattern: &str,
    files_only: bool,
    show_stats: bool,
    start: Instant,
    strategy: StrategyOverride,
) {
    let open_start = show_stats.then(Instant::now);
    let reader = match qndx_index::IndexReader::open(index_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error opening index: {}", e);
            std::process::exit(1);
        }
    };
    let open_time_ms = open_start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);

    match qndx_query::index_search_with_strategy_and_timing(
        &reader, root, pattern, strategy, show_stats,
    ) {
        Ok(result) => {
            if files_only {
                let mut seen = std::collections::BTreeSet::new();
                for m in &result.results.matches {
                    if seen.insert(&m.path) {
                        println!("{}", m.path);
                    }
                }
                if show_stats {
                    let elapsed = start.elapsed();
                    eprintln!(
                        "{} matching files (from {} candidates / {} total, strategy: {}) in {:.3}s [indexed]",
                        result.stats.verified_count,
                        result.stats.candidate_count,
                        result.stats.total_files,
                        result.stats.strategy,
                        elapsed.as_secs_f64(),
                    );
                    eprintln!(
                        "  timing: open={:.3}ms, plan={:.3}ms, candidates={:.3}ms, verify={:.3}ms",
                        open_time_ms,
                        result.stats.plan_time_ms,
                        result.stats.candidate_time_ms,
                        result.stats.verify_time_ms,
                    );
                }
            } else {
                for m in &result.results.matches {
                    println!("{}:{}:{}: {}", m.path, m.line, m.column, m.text);
                }
                if show_stats {
                    let elapsed = start.elapsed();
                    eprintln!(
                        "{} matches in {} files ({} bytes, {} candidates / {} total, {} lookups, strategy: {}) in {:.3}s [indexed]",
                        result.results.matches.len(),
                        result.results.files_scanned,
                        result.results.bytes_scanned,
                        result.stats.candidate_count,
                        result.stats.total_files,
                        result.stats.lookup_count,
                        result.stats.strategy,
                        elapsed.as_secs_f64(),
                    );
                    eprintln!(
                        "  timing: open={:.3}ms, plan={:.3}ms, candidates={:.3}ms, verify={:.3}ms",
                        open_time_ms,
                        result.stats.plan_time_ms,
                        result.stats.candidate_time_ms,
                        result.stats.verify_time_ms,
                    );
                }
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

fn run_scan_search(
    root: &Path,
    pattern: &str,
    config: &WalkConfig,
    files_only: bool,
    show_stats: bool,
    start: Instant,
) {
    if files_only {
        match scan::scan_matching_files(root, pattern, config) {
            Ok(files) => {
                for f in &files {
                    println!("{}", f);
                }
                if show_stats {
                    let elapsed = start.elapsed();
                    eprintln!(
                        "{} matching files found in {:.3}s [scan]",
                        files.len(),
                        elapsed.as_secs_f64()
                    );
                }
            }
            Err(e) => {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        match scan::scan_search(root, pattern, config) {
            Ok(results) => {
                for m in &results.matches {
                    println!("{}:{}:{}: {}", m.path, m.line, m.column, m.text);
                }
                if show_stats {
                    let elapsed = start.elapsed();
                    eprintln!(
                        "{} matches in {} files ({} bytes) in {:.3}s [scan]",
                        results.matches.len(),
                        results.files_scanned,
                        results.bytes_scanned,
                        elapsed.as_secs_f64(),
                    );
                }
            }
            Err(e) => {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
        }
    }
}
