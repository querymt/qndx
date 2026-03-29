//! Deterministic fixture datasets for benchmarking.
//!
//! All generators use a fixed seed so results are reproducible across runs.

use rand::rngs::ChaCha8Rng;
use rand::RngExt;
use rand::SeedableRng;

/// Fixed seed for all fixture generation.
const FIXTURE_SEED: u64 = 0xDEAD_BEEF_CAFE_1234;

/// A synthetic corpus file.
#[derive(Debug, Clone)]
pub struct FixtureFile {
    pub path: String,
    pub content: Vec<u8>,
}

/// A complete synthetic corpus.
#[derive(Debug, Clone)]
pub struct Corpus {
    pub name: String,
    pub files: Vec<FixtureFile>,
}

impl Corpus {
    /// Total bytes across all files.
    pub fn total_bytes(&self) -> usize {
        self.files.iter().map(|f| f.content.len()).sum()
    }
}

/// Identifier vocabulary for generating realistic source-code-like content.
const IDENTIFIERS: &[&str] = &[
    "MAX_FILE_SIZE",
    "min_buffer_len",
    "HttpResponse",
    "parse_config",
    "DatabaseConnection",
    "handle_request",
    "serialize_data",
    "NodeVisitor",
    "async_runtime",
    "thread_pool",
    "hash_map_entry",
    "BTreeMap",
    "allocator",
    "TokenStream",
    "write_output",
    "read_input",
    "transform_ast",
    "compile_module",
    "link_objects",
    "optimize_ir",
    "register_callback",
    "emit_event",
    "process_batch",
    "validate_schema",
    "render_template",
    "route_handler",
    "middleware_chain",
    "error_boundary",
    "retry_policy",
    "circuit_breaker",
    "load_balancer",
    "rate_limiter",
];

/// Keywords and operators that appear in source code.
const KEYWORDS: &[&str] = &[
    "fn ", "let ", "mut ", "pub ", "struct ", "impl ", "enum ", "match ", "if ", "else ", "for ",
    "while ", "return ", "use ", "mod ", "trait ", "const ", "static ", "async ", "await ",
    "type ", "where ",
];

const OPERATORS: &[&str] = &[
    " = ", " == ", " != ", " => ", " -> ", "::", ".", ",", ";", "(", ")", "{", "}", "[", "]",
    " + ", " - ", " * ", " / ", " & ", " | ",
];

/// Generate a single synthetic source file with the given RNG.
fn generate_source_file(rng: &mut ChaCha8Rng, approx_size: usize) -> Vec<u8> {
    let mut content = Vec::with_capacity(approx_size);

    while content.len() < approx_size {
        // Mix of identifiers, keywords, operators, and newlines
        let choice: u8 = rng.random_range(0..100);
        let segment: &[u8] = if choice < 35 {
            let idx = rng.random_range(0..IDENTIFIERS.len());
            IDENTIFIERS[idx].as_bytes()
        } else if choice < 55 {
            let idx = rng.random_range(0..KEYWORDS.len());
            KEYWORDS[idx].as_bytes()
        } else if choice < 75 {
            let idx = rng.random_range(0..OPERATORS.len());
            OPERATORS[idx].as_bytes()
        } else if choice < 85 {
            b"\n"
        } else if choice < 92 {
            b"    " // indentation
        } else {
            // Random ASCII identifier-like string
            let len = rng.random_range(3..12);
            let s: Vec<u8> = (0..len)
                .map(|_| {
                    let c = rng.random_range(0..62);
                    if c < 26 {
                        b'a' + c
                    } else if c < 52 {
                        b'A' + (c - 26)
                    } else {
                        b'0' + (c - 52)
                    }
                })
                .collect();
            content.extend_from_slice(&s);
            continue;
        };
        content.extend_from_slice(segment);
    }

    content.truncate(approx_size);
    content
}

/// Generate a small synthetic corpus (~50 files, ~500 bytes each).
pub fn small_corpus() -> Corpus {
    generate_corpus("small", 50, 500)
}

/// Generate a medium synthetic corpus (~200 files, ~5KB each).
pub fn medium_corpus() -> Corpus {
    generate_corpus("medium", 200, 5_000)
}

/// Generate a large synthetic corpus (~1000 files, ~20KB each).
pub fn large_corpus() -> Corpus {
    generate_corpus("large", 1_000, 20_000)
}

/// Generate a corpus with the given parameters using a deterministic seed.
pub fn generate_corpus(name: &str, file_count: usize, avg_file_size: usize) -> Corpus {
    let mut rng = ChaCha8Rng::seed_from_u64(FIXTURE_SEED);

    let files = (0..file_count)
        .map(|i| {
            // Vary file sizes +-50% around the average
            let size_factor: f64 = 0.5 + rng.random::<f64>();
            let size = (avg_file_size as f64 * size_factor) as usize;
            FixtureFile {
                path: format!("src/module_{:04}/file_{:04}.rs", i / 10, i),
                content: generate_source_file(&mut rng, size),
            }
        })
        .collect();

    Corpus {
        name: name.to_string(),
        files,
    }
}

/// Standard set of regex patterns for benchmarking, covering different query classes.
pub fn benchmark_patterns() -> Vec<(&'static str, &'static str)> {
    vec![
        ("literal_simple", "MAX_FILE_SIZE"),
        ("literal_underscore", "hash_map_entry"),
        ("literal_camel", "HttpResponse"),
        ("regex_alternation", "parse_config|serialize_data"),
        ("regex_class", "process_[a-z]+"),
        ("regex_wildcard", "handle_.*request"),
        ("regex_digit", r"module_\d+"),
        ("regex_word_boundary", r"\bfn\b"),
        ("regex_complex", r"(async|sync)_runtime.*pool"),
        ("regex_broad", ".*"),
    ]
}

/// Generate a manifest at various sizes for serialization benchmarks.
pub fn sample_manifests() -> Vec<(String, qndx_core::Manifest)> {
    vec![
        ("tiny".into(), make_manifest(10)),
        ("small".into(), make_manifest(100)),
        ("medium".into(), make_manifest(1_000)),
        ("large".into(), make_manifest(10_000)),
    ]
}

fn make_manifest(file_count: u32) -> qndx_core::Manifest {
    let mut rng = ChaCha8Rng::seed_from_u64(FIXTURE_SEED);
    qndx_core::Manifest {
        version: 1,
        file_count,
        ngram_count: file_count * 50,
        postings_bytes: file_count as u64 * 1024,
        base_commit: Some("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".into()),
        files: (0..file_count)
            .map(|i| {
                let depth = rng.random_range(1..=4);
                let parts: Vec<String> = (0..depth)
                    .map(|_| {
                        let idx = rng.random_range(0..IDENTIFIERS.len());
                        IDENTIFIERS[idx].to_lowercase()
                    })
                    .collect();
                format!("{}/file_{:05}.rs", parts.join("/"), i)
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// External (real) corpus support
// ---------------------------------------------------------------------------

use qndx_core::walk::WalkConfig;
use std::path::Path;

/// Configuration for loading an external corpus.
pub struct ExternalCorpusConfig {
    /// Maximum number of files to load (None = unlimited).
    pub max_files: Option<usize>,
    /// Maximum file size in bytes.
    pub max_file_size: u64,
}

impl Default for ExternalCorpusConfig {
    fn default() -> Self {
        Self {
            max_files: None,
            max_file_size: 1_048_576, // 1 MB
        }
    }
}

impl ExternalCorpusConfig {
    /// Build from environment variables.
    pub fn from_env() -> Self {
        let max_files = std::env::var("QNDX_BENCH_MAX_FILES")
            .ok()
            .and_then(|v| v.parse().ok());
        let max_file_size = std::env::var("QNDX_BENCH_MAX_FILE_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1_048_576);
        Self {
            max_files,
            max_file_size,
        }
    }
}

/// Load a real codebase from disk as a Corpus.
///
/// Walks the directory using the standard WalkConfig (respects .gitignore,
/// skips binary files, etc.). Optionally limits the number of files loaded.
pub fn external_corpus(root: &Path, config: &ExternalCorpusConfig) -> Corpus {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "external".into());

    let walk_config = WalkConfig {
        max_file_size: config.max_file_size,
        ..Default::default()
    };

    let discovered = qndx_core::walk::discover_files(root, &walk_config);

    let limit = config.max_files.unwrap_or(usize::MAX);
    let files: Vec<FixtureFile> = discovered
        .into_iter()
        .take(limit)
        .filter_map(|f| {
            let content = std::fs::read(&f.abs_path).ok()?;
            Some(FixtureFile {
                path: f.rel_path,
                content,
            })
        })
        .collect();

    Corpus { name, files }
}

/// A named benchmark pattern (name, regex).
pub struct NamedPattern {
    pub name: String,
    pub pattern: String,
}

/// Load patterns from a file.
///
/// Supports two formats per line:
/// - `name<TAB>pattern` (e.g., `find_printk\tprintk`)
/// - `pattern` alone (name is auto-generated from the pattern)
///
/// Empty lines and lines starting with `#` are skipped.
pub fn load_patterns_file(path: &Path) -> Vec<NamedPattern> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "warning: could not read patterns file {}: {}",
                path.display(),
                e
            );
            return Vec::new();
        }
    };

    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .enumerate()
        .map(|(i, line)| {
            if let Some((name, pattern)) = line.split_once('\t') {
                NamedPattern {
                    name: name.trim().to_string(),
                    pattern: pattern.trim().to_string(),
                }
            } else {
                let pat = line.trim().to_string();
                let name = if pat.len() <= 30 {
                    pat.clone()
                } else {
                    format!("pattern_{}", i)
                };
                NamedPattern { name, pattern: pat }
            }
        })
        .collect()
}

/// Get the combined set of patterns for a real corpus benchmark.
///
/// Merges generic benchmark patterns with corpus-specific patterns from a file
/// (if `QNDX_BENCH_PATTERNS` is set).
pub fn real_corpus_patterns() -> Vec<NamedPattern> {
    // Start with generic patterns
    let mut patterns: Vec<NamedPattern> = benchmark_patterns()
        .into_iter()
        .map(|(name, pat)| NamedPattern {
            name: name.to_string(),
            pattern: pat.to_string(),
        })
        .collect();

    // Add corpus-specific patterns if configured
    if let Ok(path) = std::env::var("QNDX_BENCH_PATTERNS") {
        let extra = load_patterns_file(Path::new(&path));
        if !extra.is_empty() {
            eprintln!("Loaded {} extra patterns from {}", extra.len(), path);
        }
        patterns.extend(extra);
    }

    patterns
}

/// Derive a short corpus name from a path, suitable for benchmark group names.
///
/// Uses `QNDX_BENCH_NAME` env var if set, otherwise the last path component.
pub fn corpus_bench_name(root: &Path) -> String {
    if let Ok(name) = std::env::var("QNDX_BENCH_NAME") {
        return name;
    }
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "corpus".into())
}

/// Format byte count as human-readable string.
pub fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_is_deterministic() {
        let c1 = small_corpus();
        let c2 = small_corpus();
        assert_eq!(c1.files.len(), c2.files.len());
        for (f1, f2) in c1.files.iter().zip(c2.files.iter()) {
            assert_eq!(f1.path, f2.path);
            assert_eq!(f1.content, f2.content);
        }
    }

    #[test]
    fn corpus_sizes() {
        let s = small_corpus();
        let m = medium_corpus();
        let l = large_corpus();
        assert_eq!(s.files.len(), 50);
        assert_eq!(m.files.len(), 200);
        assert_eq!(l.files.len(), 1000);
        assert!(s.total_bytes() < m.total_bytes());
        assert!(m.total_bytes() < l.total_bytes());
    }

    #[test]
    fn benchmark_patterns_nonempty() {
        let p = benchmark_patterns();
        assert!(!p.is_empty());
    }

    #[test]
    fn manifests_have_correct_file_counts() {
        for (_, m) in sample_manifests() {
            assert_eq!(m.files.len() as u32, m.file_count);
        }
    }
}
