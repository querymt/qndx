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
    /// Generic regression threshold (point estimate).
    pub regression_pct: Option<f64>,
    pub p50_regression_pct: Option<f64>,
    pub p95_regression_pct: Option<f64>,
    pub throughput_regression_pct: Option<f64>,
    pub growth_pct: Option<f64>,
    pub tolerance_pct: Option<f64>,
    pub critical: Option<bool>,
    pub description: Option<String>,
}

impl Budget {
    /// Return the effective regression threshold: regression_pct, then
    /// p50_regression_pct, then throughput_regression_pct, then 20.0 default.
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

/// Top-level budgets.toml structure.
#[derive(Debug, Deserialize)]
struct BudgetsConfig {
    /// Criterion bench name pattern -> budget key.
    #[serde(default)]
    mapping: HashMap<String, String>,
    /// CI enforcement settings.
    #[serde(default)]
    ci: CiConfig,
    /// All other top-level keys are budget groups, handled via `flatten`.
    #[serde(flatten)]
    groups: HashMap<String, toml::Value>,
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct CiConfig {
    #[serde(default = "default_true")]
    fail_on_critical: bool,
    #[serde(default = "default_true")]
    warn_on_noncritical: bool,
}

fn default_true() -> bool {
    true
}

/// Parsed comparison entry from Criterion output.
#[derive(Debug, Clone)]
struct BenchComparison {
    /// Criterion bench name, e.g. "postings_intersect_union/vec/intersect/low".
    name: String,
    /// Point-estimate change percentage (middle value from Criterion).
    change_pct: f64,
}

/// Result of checking a single benchmark against its budget.
enum CheckResult {
    Pass {
        bench: String,
        budget_key: String,
        change_pct: f64,
        threshold: f64,
    },
    Violation {
        bench: String,
        budget_key: String,
        change_pct: f64,
        threshold: f64,
        critical: bool,
    },
    Unmapped {
        bench: String,
        change_pct: f64,
    },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Check performance budgets against Criterion text output.
///
/// Returns `Ok(true)` if CI should pass, `Ok(false)` if it should fail.
pub fn check_performance_budgets(
    comparison_path: Option<&Path>,
    budgets_path: &Path,
    _fail_on_critical: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Load budgets configuration
    let budgets_content = fs::read_to_string(budgets_path)?;
    let config: BudgetsConfig = toml::from_str(&budgets_content)?;

    // Parse individual budget entries from the flattened groups.
    let budgets = parse_budget_entries(&config.groups)?;

    eprintln!("=== Performance Budget Check ===");
    eprintln!("Budgets file : {}", budgets_path.display());
    eprintln!("Mappings     : {}", config.mapping.len());
    eprintln!("Budget entries: {}", budgets.len());

    // Load comparison data
    let comparison_data = match comparison_path {
        Some(path) => {
            eprintln!("Comparison   : {}", path.display());
            fs::read_to_string(path)?
        }
        None => {
            eprintln!("Comparison   : (none provided)");
            return Err("no comparison file provided (pass --comparison bench-output.txt)".into());
        }
    };

    // Parse Criterion output
    let comparisons = parse_criterion_output(&comparison_data);
    eprintln!("Benchmarks   : {} with change data", comparisons.len());
    eprintln!();

    // Check each benchmark
    let mut results: Vec<CheckResult> = Vec::new();

    for comp in &comparisons {
        // Only check regressions (positive change = slower)
        if comp.change_pct <= 0.0 {
            continue;
        }

        // Map Criterion name -> budget key
        let budget_key = map_bench_name(&comp.name, &config.mapping);

        match budget_key {
            Some(key) => {
                if let Some(budget) = budgets.get(&key) {
                    let threshold = budget.threshold();
                    if comp.change_pct > threshold {
                        results.push(CheckResult::Violation {
                            bench: comp.name.clone(),
                            budget_key: key,
                            change_pct: comp.change_pct,
                            threshold,
                            critical: budget.is_critical(),
                        });
                    } else {
                        results.push(CheckResult::Pass {
                            bench: comp.name.clone(),
                            budget_key: key,
                            change_pct: comp.change_pct,
                            threshold,
                        });
                    }
                } else {
                    // Mapping exists but budget key not found
                    results.push(CheckResult::Unmapped {
                        bench: comp.name.clone(),
                        change_pct: comp.change_pct,
                    });
                }
            }
            None => {
                results.push(CheckResult::Unmapped {
                    bench: comp.name.clone(),
                    change_pct: comp.change_pct,
                });
            }
        }
    }

    // Tally and print
    let passed: Vec<_> = results
        .iter()
        .filter(|r| matches!(r, CheckResult::Pass { .. }))
        .collect();
    let critical_violations: Vec<_> = results
        .iter()
        .filter(|r| matches!(r, CheckResult::Violation { critical: true, .. }))
        .collect();
    let warnings: Vec<_> = results
        .iter()
        .filter(|r| {
            matches!(
                r,
                CheckResult::Violation {
                    critical: false,
                    ..
                }
            )
        })
        .collect();
    let unmapped: Vec<_> = results
        .iter()
        .filter(|r| matches!(r, CheckResult::Unmapped { .. }))
        .collect();

    for r in &passed {
        if let CheckResult::Pass {
            bench,
            budget_key,
            change_pct,
            threshold,
        } = r
        {
            eprintln!(
                "  PASS  {:.1}% <= {:.1}%  {} -> {}",
                change_pct, threshold, bench, budget_key
            );
        }
    }

    for r in &warnings {
        if let CheckResult::Violation {
            bench,
            budget_key,
            change_pct,
            threshold,
            ..
        } = r
        {
            eprintln!(
                "  WARN  {:.1}% > {:.1}%  {} -> {}",
                change_pct, threshold, bench, budget_key
            );
        }
    }

    for r in &critical_violations {
        if let CheckResult::Violation {
            bench,
            budget_key,
            change_pct,
            threshold,
            ..
        } = r
        {
            eprintln!(
                "  FAIL  {:.1}% > {:.1}%  {} -> {}",
                change_pct, threshold, bench, budget_key
            );
        }
    }

    for r in &unmapped {
        if let CheckResult::Unmapped { bench, change_pct } = r {
            eprintln!("  SKIP  {:.1}% (unmapped)  {}", change_pct, bench);
        }
    }

    eprintln!();
    eprintln!("Results:");
    eprintln!("  Passed             : {}", passed.len());
    eprintln!("  Warnings (non-crit): {}", warnings.len());
    eprintln!("  Critical violations: {}", critical_violations.len());
    eprintln!("  Unmapped (skipped) : {}", unmapped.len());
    eprintln!();

    if !critical_violations.is_empty() && config.ci.fail_on_critical {
        eprintln!(
            "FAILED: {} critical budget violation(s)",
            critical_violations.len()
        );
        return Ok(false);
    }

    if critical_violations.is_empty() && warnings.is_empty() {
        eprintln!("All performance budgets passed.");
    } else if critical_violations.is_empty() {
        eprintln!(
            "Performance budgets passed (with {} warning(s)).",
            warnings.len()
        );
    }

    Ok(true)
}

// ---------------------------------------------------------------------------
// Budget parsing
// ---------------------------------------------------------------------------

/// Parse budget entries from the flattened TOML groups.
///
/// The groups map contains entries like:
///   "postings_choice" -> Table { "intersection" -> Table { "low" -> { ... } } }
///   "serializer_choice" -> Table { "encode" -> { ... } }
///
/// We flatten these into dotted keys: "postings_choice.intersection.low" -> Budget.
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

/// Recursively collect budget entries from nested TOML tables.
fn collect_budgets(
    out: &mut HashMap<String, Budget>,
    prefix: &str,
    table: &toml::map::Map<String, toml::Value>,
) {
    // Check if this table itself is a budget entry (has "critical" key).
    if table.contains_key("critical") {
        if let Ok(budget) = Budget::deserialize(toml::Value::Table(table.clone())) {
            out.insert(prefix.to_string(), budget);
            return;
        }
    }

    // Otherwise recurse into sub-tables.
    for (key, value) in table {
        if let toml::Value::Table(sub) = value {
            let child_key = format!("{}.{}", prefix, key);
            collect_budgets(out, &child_key, sub);
        }
    }
}

// ---------------------------------------------------------------------------
// Criterion output parser
// ---------------------------------------------------------------------------

/// Parse Criterion text output into benchmark comparisons.
///
/// Handles two output formats:
///
/// Format A (multi-line, typically throughput benchmarks):
///   ```text
///   serializer_choice/postcard/encode/tiny
///                           time:   [148.50 ns 148.69 ns 148.95 ns]
///                    change:
///                           time:   [+22.639% +22.786% +22.938%] (p = ...)
///   ```
///
/// Format B (single-line, most benchmarks):
///   ```text
///   postings_intersect_union/vec/intersect/low
///                           time:   [45.678 ns 45.789 ns 45.901 ns]
///                           change: [+3.8754% +4.6023% +5.3267%] (p = ...)
///   ```
fn parse_criterion_output(data: &str) -> Vec<BenchComparison> {
    let mut comparisons = Vec::new();
    let lines: Vec<&str> = data.lines().collect();
    let mut current_bench: Option<String> = None;
    let mut saw_change_marker = false;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Detect benchmark name: a non-indented line containing '/' that
        // isn't a Criterion status message ("Benchmarking ...: Collecting/Warming/Analyzing").
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
            // Extract just the bench name (first token, or the whole line if
            // it ends before the time: field).
            let name = trimmed.split_whitespace().next().unwrap_or(trimmed);
            // Only accept names that look like bench paths (contain '/')
            if name.contains('/') {
                current_bench = Some(name.to_string());
                saw_change_marker = false;
            }
        }

        // Format B: "change: [+3.87% +4.60% +5.33%]" -- all on one line.
        if trimmed.starts_with("change:") && trimmed.contains('[') {
            if let Some(ref bench_name) = current_bench {
                if let Some(pct) = extract_point_estimate(trimmed) {
                    comparisons.push(BenchComparison {
                        name: bench_name.clone(),
                        change_pct: pct,
                    });
                }
            }
            saw_change_marker = false;
            continue;
        }

        // Format A: bare "change:" on its own line.
        if trimmed == "change:" {
            saw_change_marker = true;
            continue;
        }

        // Format A continuation: the line after "change:" has the percentages.
        if saw_change_marker && trimmed.starts_with("time:") && trimmed.contains('%') {
            if let Some(ref bench_name) = current_bench {
                if let Some(pct) = extract_point_estimate(trimmed) {
                    comparisons.push(BenchComparison {
                        name: bench_name.clone(),
                        change_pct: pct,
                    });
                }
            }
            saw_change_marker = false;
            continue;
        }

        // Reset the change marker if we see any other non-blank line.
        if saw_change_marker && !trimmed.is_empty() {
            saw_change_marker = false;
        }

        let _ = i; // suppress unused warning
    }

    comparisons
}

/// Extract the point estimate (middle value) from a bracket group like
/// `[+1.23% +4.56% +7.89%]` or `[-1.23% -0.56% +0.12%]`.
///
/// Returns the middle (2nd) percentage value.
fn extract_point_estimate(line: &str) -> Option<f64> {
    let start = line.find('[')?;
    let end = line.find(']')?;
    let bracket = &line[start + 1..end];

    let parts: Vec<&str> = bracket.split_whitespace().collect();
    if parts.len() >= 3 {
        // Middle value is the point estimate.
        let mid = parts[1].trim_end_matches('%');
        return mid.parse::<f64>().ok();
    }

    None
}

// ---------------------------------------------------------------------------
// Mapping: Criterion bench name -> budget key
// ---------------------------------------------------------------------------

/// Map a Criterion bench name to a budget key using the [mapping] table.
///
/// Patterns support simple glob matching where `*` matches any single
/// path segment (separated by `/`).
fn map_bench_name(bench_name: &str, mapping: &HashMap<String, String>) -> Option<String> {
    for (pattern, budget_key) in mapping {
        if glob_match(pattern, bench_name) {
            return Some(budget_key.clone());
        }
    }
    None
}

/// Simple glob matching: split by `/`, `*` matches any single segment,
/// `word*` matches a segment starting with `word`.
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

/// Match a single path segment against a pattern segment.
/// `*` matches anything, `foo*` matches segments starting with `foo`,
/// `*bar` matches segments ending with `bar`, exact match otherwise.
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

    #[test]
    fn parse_format_b_single_line_change() {
        let input = r#"
postings_intersect_union/vec/intersect/low
                        time:   [45.678 ns 45.789 ns 45.901 ns]
                        change: [+3.8754% +4.6023% +5.3267%] (p = 0.00 < 0.05)
                        Performance has regressed.
"#;
        let comparisons = parse_criterion_output(input);
        assert_eq!(comparisons.len(), 1);
        assert_eq!(
            comparisons[0].name,
            "postings_intersect_union/vec/intersect/low"
        );
        assert!((comparisons[0].change_pct - 4.6023).abs() < 0.001);
    }

    #[test]
    fn parse_format_a_multi_line_change() {
        let input = r#"
serializer_choice/postcard/encode/tiny
                        time:   [148.50 ns 148.69 ns 148.95 ns]
                        thrpt:  [2.70 GiB/s 2.71 GiB/s 2.71 GiB/s]
                 change:
                        time:   [+22.639% +22.786% +22.938%] (p = 0.00 < 0.05)
                        thrpt:  [-18.658% -18.558% -18.460%]
                        Performance has regressed.
"#;
        let comparisons = parse_criterion_output(input);
        assert_eq!(comparisons.len(), 1);
        assert_eq!(
            comparisons[0].name,
            "serializer_choice/postcard/encode/tiny"
        );
        assert!((comparisons[0].change_pct - 22.786).abs() < 0.001);
    }

    #[test]
    fn parse_mixed_formats() {
        let input = r#"
serializer_choice/postcard/encode/tiny
                        time:   [148.50 ns 148.69 ns 148.95 ns]
                 change:
                        time:   [+22.639% +22.786% +22.938%] (p = 0.00 < 0.05)
                        Performance has regressed.

postings_intersect_union/vec/intersect/low
                        time:   [45.678 ns 45.789 ns 45.901 ns]
                        change: [-1.23% -0.56% +0.12%] (p = 0.42 > 0.05)
                        No change in performance detected.

end_to_end_search/search/literal_simple
                        time:   [16.182 us 16.209 us 16.242 us]
                        change: [+1.0503% +1.2440% +1.4184%] (p = 0.00 < 0.05)
                        Performance has regressed.
"#;
        let comparisons = parse_criterion_output(input);
        assert_eq!(comparisons.len(), 3);
        assert!((comparisons[0].change_pct - 22.786).abs() < 0.001);
        assert!((comparisons[1].change_pct - (-0.56)).abs() < 0.001);
        assert!((comparisons[2].change_pct - 1.244).abs() < 0.001);
    }

    #[test]
    fn glob_exact() {
        assert!(glob_match(
            "git_overlay/detect_dirty_files",
            "git_overlay/detect_dirty_files"
        ));
        assert!(!glob_match(
            "git_overlay/detect_dirty_files",
            "git_overlay/head_commit"
        ));
    }

    #[test]
    fn glob_star_segment() {
        assert!(glob_match(
            "serializer_choice/*/encode/*",
            "serializer_choice/postcard/encode/tiny"
        ));
        assert!(!glob_match(
            "serializer_choice/*/encode/*",
            "serializer_choice/postcard/decode/tiny"
        ));
    }

    #[test]
    fn glob_prefix_star() {
        assert!(glob_match(
            "end_to_end_search/search/literal_*",
            "end_to_end_search/search/literal_simple"
        ));
        assert!(glob_match(
            "end_to_end_search/search/literal_*",
            "end_to_end_search/search/literal_camel"
        ));
        assert!(!glob_match(
            "end_to_end_search/search/literal_*",
            "end_to_end_search/search/regex_class"
        ));
    }

    #[test]
    fn glob_different_depth() {
        assert!(!glob_match("query_planner/plan/*", "query_planner/plan"));
        assert!(glob_match(
            "query_planner/plan/*",
            "query_planner/plan/literal_simple"
        ));
    }

    #[test]
    fn mapping_lookup() {
        let mut mapping = HashMap::new();
        mapping.insert(
            "serializer_choice/*/encode/*".to_string(),
            "serializer_choice.encode".to_string(),
        );
        mapping.insert(
            "end_to_end_search/search/regex_*".to_string(),
            "end_to_end_search.regex".to_string(),
        );
        mapping.insert(
            "git_overlay/head_commit".to_string(),
            "git_overlay.dirty_detection".to_string(),
        );

        assert_eq!(
            map_bench_name("serializer_choice/postcard/encode/tiny", &mapping),
            Some("serializer_choice.encode".to_string())
        );
        assert_eq!(
            map_bench_name("end_to_end_search/search/regex_class", &mapping),
            Some("end_to_end_search.regex".to_string())
        );
        assert_eq!(
            map_bench_name("git_overlay/head_commit", &mapping),
            Some("git_overlay.dirty_detection".to_string())
        );
        assert_eq!(map_bench_name("unknown/bench/name", &mapping), None);
    }

    #[test]
    fn extract_point_estimate_positive() {
        let line = "change: [+3.8754% +4.6023% +5.3267%] (p = 0.00 < 0.05)";
        assert!((extract_point_estimate(line).unwrap() - 4.6023).abs() < 0.001);
    }

    #[test]
    fn extract_point_estimate_negative() {
        let line = "change: [-5.1234% -4.5678% -3.9012%] (p = 0.00 < 0.05)";
        assert!((extract_point_estimate(line).unwrap() - (-4.5678)).abs() < 0.001);
    }

    #[test]
    fn budget_threshold_precedence() {
        let b = Budget {
            regression_pct: Some(25.0),
            p50_regression_pct: Some(10.0),
            p95_regression_pct: None,
            throughput_regression_pct: None,
            growth_pct: None,
            tolerance_pct: None,
            critical: Some(true),
            description: None,
        };
        // regression_pct takes precedence
        assert!((b.threshold() - 25.0).abs() < 0.001);

        let b2 = Budget {
            regression_pct: None,
            p50_regression_pct: Some(10.0),
            p95_regression_pct: None,
            throughput_regression_pct: None,
            growth_pct: None,
            tolerance_pct: None,
            critical: None,
            description: None,
        };
        // falls back to p50
        assert!((b2.threshold() - 10.0).abs() < 0.001);
        // critical defaults to false
        assert!(!b2.is_critical());
    }
}
