//! Deterministic fixture datasets for benchmarking.
//!
//! All generators use a fixed seed so results are reproducible across runs.

use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

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
        let choice: u8 = rng.gen_range(0..100);
        let segment: &[u8] = if choice < 35 {
            let idx = rng.gen_range(0..IDENTIFIERS.len());
            IDENTIFIERS[idx].as_bytes()
        } else if choice < 55 {
            let idx = rng.gen_range(0..KEYWORDS.len());
            KEYWORDS[idx].as_bytes()
        } else if choice < 75 {
            let idx = rng.gen_range(0..OPERATORS.len());
            OPERATORS[idx].as_bytes()
        } else if choice < 85 {
            b"\n"
        } else if choice < 92 {
            b"    " // indentation
        } else {
            // Random ASCII identifier-like string
            let len = rng.gen_range(3..12);
            let s: Vec<u8> = (0..len)
                .map(|_| {
                    let c = rng.gen_range(0..62);
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
            let size_factor: f64 = 0.5 + rng.gen::<f64>();
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
                let depth = rng.gen_range(1..=4);
                let parts: Vec<String> = (0..depth)
                    .map(|_| {
                        let idx = rng.gen_range(0..IDENTIFIERS.len());
                        IDENTIFIERS[idx].to_lowercase()
                    })
                    .collect();
                format!("{}/file_{:05}.rs", parts.join("/"), i)
            })
            .collect(),
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
