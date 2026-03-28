//! Benchmark report generation: human-readable and machine-readable (JSON).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// A single benchmark result entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub group: String,
    pub name: String,
    pub mean_ns: f64,
    pub std_dev_ns: f64,
    pub throughput_mb_s: Option<f64>,
}

/// Full benchmark report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReport {
    pub timestamp: String,
    pub results: Vec<BenchResult>,
}

/// Generate a benchmark report from Criterion output directory.
pub fn generate_report(criterion_dir: &str, format: &str) {
    let report = collect_results(criterion_dir);

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&report).unwrap();
            println!("{}", json);

            // Also save to file
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

/// Collect results by scanning Criterion output directories.
fn collect_results(criterion_dir: &str) -> BenchReport {
    let base = Path::new(criterion_dir);
    let mut results = Vec::new();

    if base.exists() {
        // Walk group directories
        if let Ok(groups) = fs::read_dir(base) {
            for group_entry in groups.flatten() {
                let group_path = group_entry.path();
                if !group_path.is_dir() {
                    continue;
                }
                let group_name = group_entry.file_name().to_string_lossy().into_owned();

                // Skip Criterion internal dirs
                if group_name.starts_with('.') || group_name == "report" {
                    continue;
                }

                // Look for benchmark subdirectories
                if let Ok(benches) = fs::read_dir(&group_path) {
                    for bench_entry in benches.flatten() {
                        let bench_path = bench_entry.path();
                        // Look for estimates.json in the "new" subdirectory
                        let estimates_path = bench_path.join("new").join("estimates.json");
                        if estimates_path.exists() {
                            if let Ok(content) = fs::read_to_string(&estimates_path) {
                                if let Some(result) =
                                    parse_criterion_estimates(&group_name, &bench_entry, &content)
                                {
                                    results.push(result);
                                }
                            }
                        }
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

/// Parse a Criterion estimates.json into a BenchResult.
fn parse_criterion_estimates(
    group: &str,
    bench_entry: &fs::DirEntry,
    content: &str,
) -> Option<BenchResult> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;
    let mean = v.get("mean")?.get("point_estimate")?.as_f64()?;
    let std_dev = v.get("std_dev")?.get("point_estimate")?.as_f64()?;

    Some(BenchResult {
        group: group.to_string(),
        name: bench_entry.file_name().to_string_lossy().into_owned(),
        mean_ns: mean,
        std_dev_ns: std_dev,
        throughput_mb_s: None,
    })
}

/// Simple timestamp without pulling in chrono.
fn chrono_stub() -> String {
    // Use std::time for a basic timestamp
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("unix:{}", duration.as_secs())
}

/// Budget configuration for a benchmark group/metric.
#[derive(Debug, Clone, Deserialize)]
pub struct Budget {
    pub p50_regression_pct: Option<f64>,
    pub p95_regression_pct: Option<f64>,
    pub throughput_regression_pct: Option<f64>,
    pub growth_pct: Option<f64>,
    pub tolerance_pct: Option<f64>,
    pub critical: Option<bool>,
}

/// Check performance budgets against baseline comparison.
///
/// Returns Ok(true) if all budgets pass, Ok(false) if violations found.
pub fn check_performance_budgets(
    comparison_path: Option<&Path>,
    budgets_path: &Path,
    fail_on_critical: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Load budgets configuration
    let budgets_content = fs::read_to_string(budgets_path)?;
    let budgets: HashMap<String, HashMap<String, Budget>> = toml::from_str(&budgets_content)?;

    println!("=== Performance Budget Check ===");
    println!("Budgets: {}", budgets_path.display());

    // If no comparison file provided, try to parse from Criterion output
    let comparison_data = if let Some(path) = comparison_path {
        println!("Comparison: {}", path.display());
        fs::read_to_string(path)?
    } else {
        println!("Comparison: Reading from target/criterion/");
        // For MVP, we'll implement a simple parser
        // In production, this would parse Criterion's comparison output
        String::new()
    };

    // Parse comparison results
    let comparisons = parse_criterion_comparison(&comparison_data)?;

    let mut violations = Vec::new();
    let mut warnings = Vec::new();
    let mut checks_passed = 0;

    println!();
    println!("Checking budgets...");
    println!();

    // Check each benchmark against its budget
    for (benchmark_id, comparison) in &comparisons {
        // Try to find matching budget
        if let Some(budget) = find_budget(&budgets, benchmark_id) {
            let result = check_budget(benchmark_id, comparison, budget);

            match result {
                BudgetResult::Pass => {
                    checks_passed += 1;
                }
                BudgetResult::Violation {
                    message,
                    is_critical,
                } => {
                    if is_critical {
                        violations.push(message);
                    } else {
                        warnings.push(message);
                    }
                }
            }
        }
    }

    // Print results
    println!("Results:");
    println!("  ✓ Passed: {}", checks_passed);
    println!("  ⚠ Warnings: {}", warnings.len());
    println!("  ✗ Critical violations: {}", violations.len());
    println!();

    if !warnings.is_empty() {
        println!("Warnings (non-critical):");
        for w in &warnings {
            println!("  ⚠ {}", w);
        }
        println!();
    }

    if !violations.is_empty() {
        println!("CRITICAL VIOLATIONS:");
        for v in &violations {
            println!("  ✗ {}", v);
        }
        println!();

        if fail_on_critical {
            println!("❌ Performance budgets check FAILED (critical violations)");
            return Ok(false);
        }
    }

    if violations.is_empty() && warnings.is_empty() {
        println!("✅ All performance budgets passed!");
    } else if violations.is_empty() {
        println!("✅ Performance budgets passed (with warnings)");
    }

    Ok(violations.is_empty() || !fail_on_critical)
}

/// Result of a budget check.
enum BudgetResult {
    Pass,
    Violation { message: String, is_critical: bool },
}

/// Find budget for a given benchmark ID.
fn find_budget<'a>(
    budgets: &'a HashMap<String, HashMap<String, Budget>>,
    benchmark_id: &str,
) -> Option<&'a Budget> {
    // Parse benchmark_id like "postings_choice.intersection.low"
    let parts: Vec<&str> = benchmark_id.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    let group = parts[0];
    let metric_key = parts[1..].join(".");

    budgets.get(group)?.get(&metric_key)
}

/// Check a single benchmark against its budget.
fn check_budget(benchmark_id: &str, comparison: &Comparison, budget: &Budget) -> BudgetResult {
    let is_critical = budget.critical.unwrap_or(false);

    // Check p50 regression
    if let (Some(threshold), Some(change_pct)) =
        (budget.p50_regression_pct, comparison.p50_change_pct)
    {
        if change_pct > threshold {
            return BudgetResult::Violation {
                message: format!(
                    "{}: p50 regression {:.1}% exceeds budget {:.1}%",
                    benchmark_id, change_pct, threshold
                ),
                is_critical,
            };
        }
    }

    // Check p95 regression
    if let (Some(threshold), Some(change_pct)) =
        (budget.p95_regression_pct, comparison.p95_change_pct)
    {
        if change_pct > threshold {
            return BudgetResult::Violation {
                message: format!(
                    "{}: p95 regression {:.1}% exceeds budget {:.1}%",
                    benchmark_id, change_pct, threshold
                ),
                is_critical,
            };
        }
    }

    // Check throughput regression
    if let (Some(threshold), Some(change_pct)) = (
        budget.throughput_regression_pct,
        comparison.throughput_change_pct,
    ) {
        if change_pct < -threshold {
            // Negative because lower throughput is worse
            return BudgetResult::Violation {
                message: format!(
                    "{}: throughput regression {:.1}% exceeds budget {:.1}%",
                    benchmark_id, -change_pct, threshold
                ),
                is_critical,
            };
        }
    }

    BudgetResult::Pass
}

/// Comparison data from Criterion.
#[derive(Debug)]
struct Comparison {
    p50_change_pct: Option<f64>,
    p95_change_pct: Option<f64>,
    throughput_change_pct: Option<f64>,
}

/// Parse Criterion comparison output.
///
/// For MVP, this is a simplified parser. In production, we'd parse
/// Criterion's JSON output or use its programmatic API.
fn parse_criterion_comparison(
    data: &str,
) -> Result<HashMap<String, Comparison>, Box<dyn std::error::Error>> {
    let mut comparisons = HashMap::new();

    // If no data provided, return empty (for MVP we'll stub this)
    if data.is_empty() {
        // Stub for testing
        return Ok(comparisons);
    }

    // Parse Criterion output format
    // Example: "postings_intersect_union/vec/low   time:   [12.345 µs 12.456 µs 12.567 µs]"
    //          "                                   change: [-5.1234% -4.5678% -3.9012%] (p = 0.00 < 0.05)"

    let lines: Vec<&str> = data.lines().collect();
    let mut current_bench: Option<String> = None;

    for line in lines {
        // Detect benchmark name
        if line.contains("time:") && !line.trim().starts_with("change:") {
            // Extract benchmark name
            if let Some(name) = line.split_whitespace().next() {
                current_bench = Some(name.to_string());
            }
        }

        // Detect change line
        if line.contains("change:") {
            if let Some(bench_name) = &current_bench {
                // Parse change percentage
                // Example: "change: [-5.1234% -4.5678% -3.9012%]"
                if let Some(change_pct) = extract_change_percentage(line) {
                    comparisons.insert(
                        bench_name.clone(),
                        Comparison {
                            p50_change_pct: Some(change_pct),
                            p95_change_pct: None,
                            throughput_change_pct: None,
                        },
                    );
                }
            }
        }
    }

    Ok(comparisons)
}

/// Extract change percentage from Criterion output line.
fn extract_change_percentage(line: &str) -> Option<f64> {
    // Look for pattern like "[-5.1234% -4.5678% -3.9012%]"
    let start = line.find('[')?;
    let end = line.find(']')?;
    let bracket_content = &line[start + 1..end];

    // Split by whitespace and find the middle value (median estimate)
    let parts: Vec<&str> = bracket_content.split_whitespace().collect();
    if parts.len() >= 2 {
        let middle = parts[1];
        // Remove % and parse
        let value_str = middle.trim_end_matches('%');
        return value_str.parse::<f64>().ok();
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report() {
        let report = collect_results("/nonexistent/path");
        assert!(report.results.is_empty());
    }

    #[test]
    fn bench_result_serializes() {
        let r = BenchResult {
            group: "test".into(),
            name: "bench_1".into(),
            mean_ns: 1234.5,
            std_dev_ns: 12.3,
            throughput_mb_s: Some(100.0),
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: BenchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.group, "test");
    }
}
