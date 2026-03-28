//! Decompose a regex pattern into literal segments for n-gram lookup.

use qndx_core::NgramHash;
use qndx_index::extract_trigrams;

/// Result of decomposing a regex into n-gram lookups.
#[derive(Debug, Clone)]
pub struct Decomposition {
    /// Required n-gram hashes (all must match — AND semantics).
    pub required: Vec<NgramHash>,
    /// Alternative branches (any branch can match — OR semantics).
    /// Each branch is a set of required hashes.
    pub alternatives: Vec<Vec<NgramHash>>,
}

/// Extract literal segments from a regex pattern string.
/// This is a simplified heuristic: extract runs of literal bytes.
pub fn decompose_pattern(pattern: &str) -> Decomposition {
    let literals = extract_literals(pattern);
    let mut required = Vec::new();

    for lit in &literals {
        let trigrams = extract_trigrams(lit.as_bytes());
        required.extend(trigrams);
    }

    required.sort_unstable();
    required.dedup();

    Decomposition {
        required,
        alternatives: Vec::new(),
    }
}

/// Simple literal extraction: pull out runs of non-meta characters.
fn extract_literals(pattern: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut current = String::new();
    let mut chars = pattern.chars().peekable();
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if escaped {
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
        } else if ".*+?|()[]{}^$".contains(ch) {
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
}
