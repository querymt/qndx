//! qndx CLI: index, search, bench, and report commands.

use clap::{Parser, Subcommand};

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
    /// Search using regex
    Search {
        /// Regex pattern to search for
        pattern: String,
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
        Commands::Search { pattern } => {
            println!("Search for '{}' not yet implemented (M1)", pattern);
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
