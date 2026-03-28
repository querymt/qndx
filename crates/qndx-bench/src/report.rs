//! Benchmark report generation: human-readable and machine-readable (JSON).

use serde::{Deserialize, Serialize};
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
