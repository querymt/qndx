//! qndx CLI: index, search, bench, and report commands.

use clap::{Parser, Subcommand};
use qndx_core::scan;
use qndx_core::walk::WalkConfig;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "qndx", about = "Fast regex search indexer for large repositories")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build the search index
    Index,
    /// Search using regex (scan-only for now)
    Search {
        /// Regex pattern to search for
        pattern: String,
        /// Root directory to search (defaults to current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,
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
        Commands::Index => {
            println!("Index building not yet implemented (M2)");
        }
        Commands::Search {
            pattern,
            root,
            max_file_size,
            hidden,
            binary,
            files_only,
            stats,
        } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            let config = WalkConfig {
                max_file_size,
                include_hidden: hidden,
                skip_binary: !binary,
                ..Default::default()
            };

            let start = Instant::now();

            if files_only {
                match scan::scan_matching_files(&root, &pattern, &config) {
                    Ok(files) => {
                        for f in &files {
                            println!("{}", f);
                        }
                        if stats {
                            let elapsed = start.elapsed();
                            eprintln!(
                                "{} matching files found in {:.3}s",
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
                match scan::scan_search(&root, &pattern, &config) {
                    Ok(results) => {
                        for m in &results.matches {
                            println!("{}:{}:{}: {}", m.path, m.line, m.column, m.text);
                        }
                        if stats {
                            let elapsed = start.elapsed();
                            eprintln!(
                                "{} matches in {} files ({} bytes) in {:.3}s",
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
