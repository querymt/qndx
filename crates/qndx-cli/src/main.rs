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
    /// Show index statistics (ngram/postings distribution)
    Stats {
        /// Root directory (used with default index path)
        #[arg(short, long)]
        root: Option<PathBuf>,
        /// Index directory
        #[arg(short, long)]
        index_dir: Option<PathBuf>,
    },
    /// Benchmark operations
    #[cfg(feature = "bench-tools")]
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

#[cfg(feature = "bench-tools")]
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

#[cfg(feature = "bench-tools")]
mod bench_tools {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct BenchResult {
        group: String,
        name: String,
        mean_ns: f64,
        std_dev_ns: f64,
        throughput_mb_s: Option<f64>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct BenchReport {
        timestamp: String,
        results: Vec<BenchResult>,
    }

    pub fn generate_report(criterion_dir: &str, format: &str) {
        let report = collect_results(criterion_dir);
        match format {
            "json" => {
                let json = serde_json::to_string_pretty(&report).unwrap();
                println!("{}", json);
                let out_dir = Path::new("benchmarks/results");
                if fs::create_dir_all(out_dir).is_ok() {
                    let path = out_dir.join("latest.json");
                    let _ = fs::write(&path, &json);
                    eprintln!("Report saved to {}", path.display());
                }
            }
            _ => {
                println!("=== qndx Benchmark Report ===");
                println!("Timestamp: {}", report.timestamp);
                println!();
                if report.results.is_empty() {
                    println!("No benchmark results found in '{}'.", criterion_dir);
                    println!("Run `cargo bench` first to generate results.");
                    return;
                }
                let mut current_group = String::new();
                for r in &report.results {
                    if r.group != current_group {
                        println!("--- {} ---", r.group);
                        current_group = r.group.clone();
                    }
                    print!("  {:<40} {:>12.2} ns", r.name, r.mean_ns);
                    if let Some(tp) = r.throughput_mb_s {
                        print!("  ({:.1} MB/s)", tp);
                    }
                    println!("  +/- {:.2} ns", r.std_dev_ns);
                }
                println!();
                println!("Total benchmarks: {}", report.results.len());
            }
        }
    }

    pub fn check_performance_budgets(
        comparison_path: Option<&Path>,
        budgets_path: &Path,
        _fail_on_critical: bool,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let budgets_content = fs::read_to_string(budgets_path)?;
        let config: BudgetsConfig = toml::from_str(&budgets_content)?;
        let budgets = parse_budget_entries(&config.groups)?;

        let comparison_data = match comparison_path {
            Some(path) => fs::read_to_string(path)?,
            None => {
                return Err(
                    "no comparison file provided (pass --comparison bench-output.txt)".into(),
                );
            }
        };

        let comparisons = parse_criterion_output(&comparison_data);
        let mut critical_violations = 0;

        for comp in &comparisons {
            if comp.change_pct <= 0.0 {
                continue;
            }
            if let Some(key) = map_bench_name(&comp.name, &config.mapping)
                && let Some(budget) = budgets.get(&key)
            {
                let threshold = budget.threshold();
                if comp.change_pct > threshold {
                    if budget.is_critical() {
                        critical_violations += 1;
                    }
                    eprintln!(
                        "{} {:.1}% > {:.1}% {} -> {}",
                        if budget.is_critical() { "FAIL" } else { "WARN" },
                        comp.change_pct,
                        threshold,
                        comp.name,
                        key
                    );
                }
            }
        }

        if critical_violations > 0 && config.ci.fail_on_critical {
            return Ok(false);
        }
        Ok(true)
    }

    fn collect_results(criterion_dir: &str) -> BenchReport {
        let base = Path::new(criterion_dir);
        let mut results = Vec::new();
        if base.exists()
            && let Ok(groups) = fs::read_dir(base)
        {
            for group_entry in groups.flatten() {
                let group_path = group_entry.path();
                if !group_path.is_dir() {
                    continue;
                }
                let group_name = group_entry.file_name().to_string_lossy().into_owned();
                if group_name.starts_with('.') || group_name == "report" {
                    continue;
                }
                if let Ok(benches) = fs::read_dir(&group_path) {
                    for bench_entry in benches.flatten() {
                        let estimates_path = bench_entry.path().join("new").join("estimates.json");
                        if estimates_path.exists()
                            && let Ok(content) = fs::read_to_string(&estimates_path)
                            && let Some(result) =
                                parse_criterion_estimates(&group_name, &bench_entry, &content)
                        {
                            results.push(result);
                        }
                    }
                }
            }
        }
        results.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
        BenchReport {
            timestamp: chrono_stub(),
            results,
        }
    }

    fn parse_criterion_estimates(
        group: &str,
        bench_entry: &fs::DirEntry,
        content: &str,
    ) -> Option<BenchResult> {
        let v: serde_json::Value = serde_json::from_str(content).ok()?;
        Some(BenchResult {
            group: group.to_string(),
            name: bench_entry.file_name().to_string_lossy().into_owned(),
            mean_ns: v.get("mean")?.get("point_estimate")?.as_f64()?,
            std_dev_ns: v.get("std_dev")?.get("point_estimate")?.as_f64()?,
            throughput_mb_s: None,
        })
    }

    fn chrono_stub() -> String {
        let duration = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        format!("unix:{}", duration.as_secs())
    }

    #[derive(Debug, Clone, Deserialize)]
    struct Budget {
        regression_pct: Option<f64>,
        p50_regression_pct: Option<f64>,
        throughput_regression_pct: Option<f64>,
        critical: Option<bool>,
    }

    impl Budget {
        fn threshold(&self) -> f64 {
            self.regression_pct
                .or(self.p50_regression_pct)
                .or(self.throughput_regression_pct)
                .unwrap_or(20.0)
        }
        fn is_critical(&self) -> bool {
            self.critical.unwrap_or(false)
        }
    }

    #[derive(Debug, Deserialize)]
    struct BudgetsConfig {
        #[serde(default)]
        mapping: HashMap<String, String>,
        #[serde(default)]
        ci: CiConfig,
        #[serde(flatten)]
        groups: HashMap<String, toml::Value>,
    }

    #[derive(Debug, Deserialize, Default)]
    struct CiConfig {
        #[serde(default = "default_true")]
        fail_on_critical: bool,
    }

    fn default_true() -> bool {
        true
    }

    #[derive(Debug, Clone)]
    struct BenchComparison {
        name: String,
        change_pct: f64,
    }

    fn parse_budget_entries(
        groups: &HashMap<String, toml::Value>,
    ) -> Result<HashMap<String, Budget>, Box<dyn std::error::Error>> {
        let mut budgets = HashMap::new();
        for (group_name, group_value) in groups {
            if let toml::Value::Table(table) = group_value {
                collect_budgets(&mut budgets, group_name, table);
            }
        }
        Ok(budgets)
    }

    fn collect_budgets(
        out: &mut HashMap<String, Budget>,
        prefix: &str,
        table: &toml::map::Map<String, toml::Value>,
    ) {
        if table.contains_key("critical")
            && let Ok(budget) = Budget::deserialize(toml::Value::Table(table.clone()))
        {
            out.insert(prefix.to_string(), budget);
            return;
        }
        for (key, value) in table {
            if let toml::Value::Table(sub) = value {
                let child_key = format!("{}.{}", prefix, key);
                collect_budgets(out, &child_key, sub);
            }
        }
    }

    fn parse_criterion_output(data: &str) -> Vec<BenchComparison> {
        let mut comparisons = Vec::new();
        let lines: Vec<&str> = data.lines().collect();
        let mut current_bench: Option<String> = None;
        let mut saw_change_marker = false;

        for line in lines {
            let trimmed = line.trim();
            if !trimmed.is_empty()
                && !trimmed.starts_with("time:")
                && !trimmed.starts_with("change:")
                && !trimmed.starts_with("thrpt:")
                && !trimmed.starts_with("Performance")
                && !trimmed.starts_with("Benchmarking")
                && !trimmed.starts_with("Found")
                && !trimmed.starts_with("Warning")
                && trimmed.contains('/')
            {
                let name = trimmed.split_whitespace().next().unwrap_or(trimmed);
                if name.contains('/') {
                    current_bench = Some(name.to_string());
                    saw_change_marker = false;
                }
            }

            if trimmed.starts_with("change:") && trimmed.contains('[') {
                if let Some(ref bench_name) = current_bench
                    && let Some(pct) = extract_point_estimate(trimmed)
                {
                    comparisons.push(BenchComparison {
                        name: bench_name.clone(),
                        change_pct: pct,
                    });
                }
                saw_change_marker = false;
                continue;
            }

            if trimmed == "change:" {
                saw_change_marker = true;
                continue;
            }

            if saw_change_marker && trimmed.starts_with("time:") && trimmed.contains('%') {
                if let Some(ref bench_name) = current_bench
                    && let Some(pct) = extract_point_estimate(trimmed)
                {
                    comparisons.push(BenchComparison {
                        name: bench_name.clone(),
                        change_pct: pct,
                    });
                }
                saw_change_marker = false;
            }
        }

        comparisons
    }

    fn extract_point_estimate(line: &str) -> Option<f64> {
        let start = line.find('[')?;
        let end = line.find(']')?;
        let parts: Vec<&str> = line[start + 1..end].split_whitespace().collect();
        if parts.len() >= 3 {
            return parts[1].trim_end_matches('%').parse::<f64>().ok();
        }
        None
    }

    fn map_bench_name(bench_name: &str, mapping: &HashMap<String, String>) -> Option<String> {
        for (pattern, budget_key) in mapping {
            if glob_match(pattern, bench_name) {
                return Some(budget_key.clone());
            }
        }
        None
    }

    fn glob_match(pattern: &str, name: &str) -> bool {
        let pat_parts: Vec<&str> = pattern.split('/').collect();
        let name_parts: Vec<&str> = name.split('/').collect();
        if pat_parts.len() != name_parts.len() {
            return false;
        }
        pat_parts
            .iter()
            .zip(name_parts.iter())
            .all(|(p, n)| segment_match(p, n))
    }

    fn segment_match(pattern: &str, segment: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        if let Some(prefix) = pattern.strip_suffix('*') {
            return segment.starts_with(prefix);
        }
        if let Some(suffix) = pattern.strip_prefix('*') {
            return segment.ends_with(suffix);
        }
        pattern == segment
    }
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
        Commands::Stats { root, index_dir } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            let index_dir = index_dir.unwrap_or_else(|| root.join(DEFAULT_INDEX_DIR));
            run_index_stats(&index_dir);
        }
        #[cfg(feature = "bench-tools")]
        Commands::Bench { action } => match action {
            BenchAction::Report {
                format,
                criterion_dir,
            } => {
                bench_tools::generate_report(&criterion_dir, &format);
            }
            BenchAction::CheckBudgets {
                comparison,
                budgets,
                fail_on_critical,
            } => {
                match bench_tools::check_performance_budgets(
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

fn run_index_stats(index_dir: &Path) {
    let reader = match qndx_index::IndexReader::open(index_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error opening index: {}", e);
            std::process::exit(1);
        }
    };

    let mut posting_lens = reader.all_posting_lens();
    if posting_lens.is_empty() {
        println!("Index Statistics ({})", index_dir.display());
        println!("  Files:              {}", reader.file_count());
        println!("  Total n-grams:      0");
        println!("  Trigram-only:       0");
        println!("  Sparse:             0");
        println!();
        println!("Posting List Distribution:");
        println!("  Mean size:          0.00");
        println!("  Median size:        0");
        println!("  P95 size:           0");
        println!("  P99 size:           0");
        println!("  Max size:           0");
        println!("  Lists > 1000:       0");
        println!("  Lists > 10000:      0");
        return;
    }

    posting_lens.sort_unstable();
    let count = posting_lens.len();
    let sum: usize = posting_lens.iter().sum();
    let mean = sum as f64 / count as f64;
    let median = posting_lens[count / 2];
    let p95 = posting_lens[((count.saturating_sub(1)) * 95) / 100];
    let p99 = posting_lens[((count.saturating_sub(1)) * 99) / 100];
    let max = *posting_lens.last().unwrap_or(&0);
    let gt_1k = posting_lens.iter().filter(|&&len| len > 1_000).count();
    let gt_10k = posting_lens.iter().filter(|&&len| len > 10_000).count();

    println!("Index Statistics ({})", index_dir.display());
    println!("  Files:              {}", reader.file_count());
    println!("  Total n-grams:      {}", reader.ngram_count());
    println!("  Trigram-only:       {}", reader.trigram_only_count());
    println!("  Sparse:             {}", reader.sparse_count());
    println!();
    println!("Posting List Distribution:");
    println!("  Mean size:          {:.2}", mean);
    println!("  Median size:        {}", median);
    println!("  P95 size:           {}", p95);
    println!("  P99 size:           {}", p99);
    println!("  Max size:           {}", max);
    println!("  Lists > 1000:       {}", gt_1k);
    println!("  Lists > 10000:      {}", gt_10k);
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
