//! Decompose a regex pattern into literal segments for n-gram lookup.
//!
//! Handles top-level alternation (`a|b`) by producing separate branches
//! with OR semantics. Within each branch, extracted trigrams use AND semantics.
//!
//! Also produces a sparse n-gram decomposition that can cover the same literals
//! with fewer, longer n-grams for reduced posting lookups.

use qndx_core::NgramHash;
use qndx_index::ngram::{extract_sparse_ngrams, extract_trigrams};

/// A sparse n-gram with its hash and the byte length of the original gram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SparseGram {
    pub hash: NgramHash,
    pub gram_len: usize,
}

/// Result of decomposing a regex into n-gram lookups.
#[derive(Debug, Clone)]
pub struct Decomposition {
    /// Required n-gram hashes (all must match — AND semantics).
    /// Used when the pattern has no top-level alternation.
    pub required: Vec<NgramHash>,
    /// Alternative branches (any branch can match — OR semantics).
    /// Each branch is a set of required hashes (AND within branch).
    /// Used when the pattern contains top-level alternation.
    pub alternatives: Vec<Vec<NgramHash>>,
    /// Sparse n-gram covering for required literals (AND semantics).
    /// Each entry covers a contiguous span of the literal; together they
    /// cover all positions, so using these instead of trigrams is sound.
    pub sparse_required: Vec<SparseGram>,
    /// Sparse n-gram covering for each alternative branch.
    pub sparse_alternatives: Vec<Vec<SparseGram>>,
}

/// Decompose a regex pattern into n-gram lookups.
///
/// - If the pattern contains top-level alternation (`|`), each branch
///   becomes a separate alternative (OR semantics between branches,
///   AND semantics within each branch).
/// - Otherwise, all extracted trigrams are required (AND semantics).
///
/// Also produces a sparse n-gram decomposition that covers the same literals
/// with potentially fewer, longer n-grams.
pub fn decompose_pattern(pattern: &str) -> Decomposition {
    let branches = split_top_level_alternation(pattern);

    if branches.len() == 1 {
        // No alternation: all trigrams are required (AND)
        let literals = extract_literals(&branches[0]);
        let mut required = Vec::new();
        let mut sparse_required = Vec::new();
        for lit in &literals {
            let trigrams = extract_trigrams(lit.as_bytes());
            required.extend(trigrams);

            let sparse = extract_sparse_ngrams(lit.as_bytes());
            for (hash, gram_len) in sparse {
                sparse_required.push(SparseGram { hash, gram_len });
            }
        }
        required.sort_unstable();
        required.dedup();
        sparse_required.sort_unstable();
        sparse_required.dedup();

        Decomposition {
            required,
            alternatives: Vec::new(),
            sparse_required,
            sparse_alternatives: Vec::new(),
        }
    } else {
        // Top-level alternation: each branch is an alternative (OR between branches)
        let mut alternatives = Vec::new();
        let mut sparse_alternatives = Vec::new();
        for branch in &branches {
            let literals = extract_literals(branch);
            let mut hashes = Vec::new();
            let mut sparse_branch = Vec::new();
            for lit in &literals {
                let trigrams = extract_trigrams(lit.as_bytes());
                hashes.extend(trigrams);

                let sparse = extract_sparse_ngrams(lit.as_bytes());
                for (hash, gram_len) in sparse {
                    sparse_branch.push(SparseGram { hash, gram_len });
                }
            }
            hashes.sort_unstable();
            hashes.dedup();
            sparse_branch.sort_unstable();
            sparse_branch.dedup();
            alternatives.push(hashes);
            sparse_alternatives.push(sparse_branch);
        }

        Decomposition {
            required: Vec::new(),
            alternatives,
            sparse_required: Vec::new(),
            sparse_alternatives,
        }
    }
}

/// Select a minimal sparse covering set from available sparse n-grams.
///
/// Given a list of sparse n-grams (each covering some byte span of the literal),
/// and the trigram hashes for the same literal, returns a subset of sparse grams
/// that fully covers the literal with fewer lookups than the trigram set.
///
/// If the sparse set cannot cover the literal (e.g., all grams are the same length
/// as trigrams), returns None, signaling a fallback to trigrams.
pub fn sparse_covering(sparse: &[SparseGram], trigram_count: usize) -> Option<Vec<SparseGram>> {
    if sparse.is_empty() {
        return None;
    }

    // The sparse n-grams already form a valid covering from the extraction algorithm.
    // We only prefer sparse if it requires fewer lookups than the trigram path.
    if sparse.len() < trigram_count {
        Some(sparse.to_vec())
    } else {
        None // No benefit — fall back to trigrams
    }
}

/// Extract literal segments from a pattern for diagnostic display.
///
/// This is a public wrapper around `extract_literals` for use by the planner
/// diagnostics. For patterns with alternation, literals from all branches are
/// returned.
pub fn extract_literals_for_diagnostics(pattern: &str) -> Vec<String> {
    let branches = split_top_level_alternation(pattern);
    let mut all_literals = Vec::new();
    for branch in &branches {
        all_literals.extend(extract_literals(branch));
    }
    all_literals
}

/// Split a pattern on top-level `|` (not inside groups/brackets).
fn split_top_level_alternation(pattern: &str) -> Vec<String> {
    let mut branches = Vec::new();
    let mut current = String::new();
    let mut depth = 0u32; // paren/bracket nesting depth
    let mut escaped = false;
    let mut in_bracket = false;

    for ch in pattern.chars() {
        if escaped {
            current.push('\\');
            current.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' => {
                escaped = true;
            }
            '[' if !in_bracket => {
                in_bracket = true;
                current.push(ch);
            }
            ']' if in_bracket => {
                in_bracket = false;
                current.push(ch);
            }
            '(' if !in_bracket => {
                depth += 1;
                current.push(ch);
            }
            ')' if !in_bracket => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            '|' if depth == 0 && !in_bracket => {
                branches.push(current.clone());
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }

    // Handle trailing escape
    if escaped {
        current.push('\\');
    }

    branches.push(current);
    branches
}

/// Simple literal extraction: pull out runs of non-meta characters.
/// Does NOT split on `|` (alternation is handled at a higher level).
/// Skips content inside character classes `[...]`.
fn extract_literals(pattern: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut current = String::new();
    let chars = pattern.chars();
    let mut escaped = false;
    let mut in_bracket = false;

    for ch in chars {
        if escaped {
            if in_bracket {
                // Inside a bracket expression — skip everything
                escaped = false;
                continue;
            }
            // After backslash, only include actual literal characters
            match ch {
                'w' | 'W' | 'd' | 'D' | 's' | 'S' | 'b' | 'B' => {
                    // Character class shorthand — not a literal
                    if current.len() >= 3 {
                        literals.push(current.clone());
                    }
                    current.clear();
                }
                _ => {
                    current.push(ch);
                }
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if in_bracket {
            // Skip all content inside character class brackets
            if ch == ']' {
                in_bracket = false;
            }
            // Don't add bracket content to current literal
        } else if ch == '[' {
            // Entering a character class — flush current literal
            if current.len() >= 3 {
                literals.push(current.clone());
            }
            current.clear();
            in_bracket = true;
        } else if ".*+?(){}^$".contains(ch) {
            // Note: '|' is NOT here; alternation is handled by split_top_level_alternation
            // Note: '[' and ']' are handled above
            if current.len() >= 3 {
                literals.push(current.clone());
            }
            current.clear();
        } else {
            current.push(ch);
        }
    }
    if current.len() >= 3 {
        literals.push(current);
    }

    literals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_pattern() {
        let d = decompose_pattern("MAX_FILE_SIZE");
        assert!(!d.required.is_empty());
        assert!(d.alternatives.is_empty());
    }

    #[test]
    fn pattern_with_metacharacters() {
        let d = decompose_pattern("foo.*bar");
        // Should extract trigrams from "foo" and "bar"
        assert!(!d.required.is_empty());
    }

    #[test]
    fn short_pattern_no_trigrams() {
        let d = decompose_pattern("ab");
        assert!(d.required.is_empty());
    }

    #[test]
    fn alternation_produces_branches() {
        let d = decompose_pattern("parse_config|serialize_data");
        // Should have no required (AND) trigrams
        assert!(d.required.is_empty());
        // Should have 2 alternative branches
        assert_eq!(d.alternatives.len(), 2);
        // Each branch should have trigrams
        assert!(!d.alternatives[0].is_empty());
        assert!(!d.alternatives[1].is_empty());
    }

    #[test]
    fn alternation_three_branches() {
        let d = decompose_pattern("foo|bar|baz");
        assert!(d.required.is_empty());
        assert_eq!(d.alternatives.len(), 3);
    }

    #[test]
    fn nested_alternation_not_split() {
        // Alternation inside parens should NOT be split at top level
        let d = decompose_pattern("(foo|bar)baz");
        // This has no top-level `|`, so everything goes to required
        assert!(d.alternatives.is_empty());
        // "baz" should produce trigrams
        assert!(!d.required.is_empty());
    }

    #[test]
    fn alternation_with_short_branch() {
        let d = decompose_pattern("ab|parse_config");
        assert!(d.required.is_empty());
        assert_eq!(d.alternatives.len(), 2);
        // "ab" is too short for trigrams
        assert!(d.alternatives[0].is_empty());
        // "parse_config" should have trigrams
        assert!(!d.alternatives[1].is_empty());
    }
}
