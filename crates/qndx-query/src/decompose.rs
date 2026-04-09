//! Decompose a regex pattern into literal segments for n-gram lookup.
//!
//! Handles top-level alternation (`a|b`) by producing separate branches
//! with OR semantics. Within each branch, extracted trigrams use AND semantics.
//!
//! Also produces a sparse n-gram decomposition that can cover the same literals
//! with fewer, longer n-grams for reduced posting lookups.

use qndx_core::NgramHash;
use qndx_index::ngram::extract_trigrams;

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
    /// Literal runs contributing to `required` hashes.
    pub required_literals: Vec<String>,
    /// Literal runs for each alternative branch.
    pub alternative_literals: Vec<Vec<String>>,
    /// Backward-compatible sparse fields. These are intentionally left empty
    /// during decomposition; sparse grams are computed lazily by the planner.
    pub sparse_required: Vec<SparseGram>,
    /// Backward-compatible sparse fields. Computed lazily by the planner.
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
        let required_literals = extract_literals(&branches[0]);
        let mut required = Vec::new();
        for lit in &required_literals {
            let trigrams = extract_trigrams(lit.as_bytes());
            required.extend(trigrams);
        }
        required.sort_unstable();
        required.dedup();

        Decomposition {
            required,
            alternatives: Vec::new(),
            required_literals,
            alternative_literals: Vec::new(),
            sparse_required: Vec::new(),
            sparse_alternatives: Vec::new(),
        }
    } else {
        // Top-level alternation: each branch is an alternative (OR between branches)
        let mut alternatives = Vec::new();
        let mut alternative_literals = Vec::new();
        for branch in &branches {
            let literals = extract_literals(branch);
            let mut hashes = Vec::new();
            for lit in &literals {
                let trigrams = extract_trigrams(lit.as_bytes());
                hashes.extend(trigrams);
            }
            hashes.sort_unstable();
            hashes.dedup();
            alternatives.push(hashes);
            alternative_literals.push(literals);
        }

        Decomposition {
            required: Vec::new(),
            alternatives,
            required_literals: Vec::new(),
            alternative_literals,
            sparse_required: Vec::new(),
            sparse_alternatives: Vec::new(),
        }
    }
}

/// Select a sparse covering candidate set from available sparse n-grams.
///
/// We keep this deterministic and conservative:
/// - If no sparse grams exist, return `None`.
/// - If sparse requires substantially more lookups than trigrams, return `None`.
/// - Otherwise, return the extracted sparse covering as-is.
///
/// This provides a robust pre-filter while the planner applies a richer cost model.
const MAX_SPARSE_LOOKUP_OVERAGE: usize = 0;

pub fn sparse_covering(sparse: &[SparseGram], trigram_count: usize) -> Option<Vec<SparseGram>> {
    if sparse.is_empty() {
        return None;
    }

    if trigram_count > 0 && sparse.len() > trigram_count + MAX_SPARSE_LOOKUP_OVERAGE {
        return None;
    }

    Some(sparse.to_vec())
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

    #[test]
    fn sparse_covering_rejects_large_overage() {
        let sparse = vec![
            SparseGram {
                hash: 1,
                gram_len: 3,
            },
            SparseGram {
                hash: 2,
                gram_len: 3,
            },
            SparseGram {
                hash: 3,
                gram_len: 3,
            },
            SparseGram {
                hash: 4,
                gram_len: 3,
            },
        ];
        assert!(sparse_covering(&sparse, 2).is_none());
        assert!(sparse_covering(&sparse, 3).is_none());
        assert!(sparse_covering(&sparse, 4).is_some());
    }
}
