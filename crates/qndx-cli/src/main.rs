//! qndx CLI: index, search, bench, and report commands.

use clap::{Parser, Subcommand};
use qndx_core::scan;
use qndx_core::walk::WalkConfig;
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
    },
    /// Benchmark operations
    Bench {
        #[command(subcommand)]
        action: BenchAction,
    },
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
        } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            let config = WalkConfig {
                max_file_size,
                include_hidden: hidden,
                skip_binary: !binary,
                ..Default::default()
            };

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
                run_index_search(&root, &idx_dir, &pattern, files_only, stats, start);
            } else {
                // Scan-only search
                run_scan_search(&root, &pattern, &config, files_only, stats, start);
            }
        }
        Commands::Bench { action } => match action {
            BenchAction::Report {
                format,
                criterion_dir,
            } => {
                qndx_bench::report::generate_report(&criterion_dir, &format);
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
) {
    match qndx_query::index_search(root, index_dir, pattern) {
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
                        "{} matching files (from {} candidates / {} total) in {:.3}s [indexed]",
                        result.stats.verified_count,
                        result.stats.candidate_count,
                        result.stats.total_files,
                        elapsed.as_secs_f64(),
                    );
                }
            } else {
                for m in &result.results.matches {
                    println!("{}:{}:{}: {}", m.path, m.line, m.column, m.text);
                }
                if show_stats {
                    let elapsed = start.elapsed();
                    eprintln!(
                        "{} matches in {} files ({} bytes, {} candidates / {} total, {} lookups) in {:.3}s [indexed]",
                        result.results.matches.len(),
                        result.results.files_scanned,
                        result.results.bytes_scanned,
                        result.stats.candidate_count,
                        result.stats.total_files,
                        result.stats.lookup_count,
                        elapsed.as_secs_f64(),
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
