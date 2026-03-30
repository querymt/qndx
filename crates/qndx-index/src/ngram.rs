//! N-gram extraction: trigram and sparse n-gram algorithms.
//!
//! Sparse n-grams use two distinct extraction modes (see the Cursor blog post
//! "Fast regex search: indexing text for agent tools" for background):
//!
//! - **`build_all`** ([`extract_sparse_ngrams_all`]): extracts *every* substring
//!   whose boundary pair-weights exceed all interior pair-weights. Used at
//!   **index time** to ensure comprehensive coverage.
//!
//! - **`build_covering`** ([`extract_sparse_ngrams_covering`]): extracts a
//!   *minimal covering set* via a monotone-stack partitioning. Used at **query
//!   time** for fewer posting lookups.
//!
//! The correctness invariant is: for any literal `L` that appears in file
//! content `F`, `covering(L) ⊆ all(F)`. This holds because `all(F)` emits
//! every qualifying substring, and the pair-weights of `L` within `F` are
//! identical to those of `L` in isolation.

use qndx_core::{hash_ngram, NgramHash};

/// Extract all overlapping trigrams from a byte slice.
/// Returns a sorted, deduplicated list of trigram hashes.
pub fn extract_trigrams(data: &[u8]) -> Vec<NgramHash> {
    if data.len() < 3 {
        return Vec::new();
    }
    let mut hashes: Vec<NgramHash> = data.windows(3).map(hash_ngram).collect();
    hashes.sort_unstable();
    hashes.dedup();
    hashes
}

/// **`build_all`** — Extract *every* sparse n-gram from a byte slice.
///
/// A sparse n-gram is a substring `data[i..j+2]` where the pair-weights at
/// positions `i` and `j` are both strictly greater than every interior
/// pair-weight at positions `i+1 ..= j-1`.
///
/// This function is intended for **index-time** use. It produces more n-grams
/// than trigram extraction (roughly 1.5–2× for typical source code), but
/// guarantees that any covering gram extracted from a substring will also
/// appear in this set.
///
/// Complexity: O(n²) worst-case, but the early-break on `interior_max >=
/// weights[i]` keeps practical cost close to O(n) for inputs with varied
/// pair-weights (e.g. source code hashed with CRC32).
///
/// Returns a sorted, deduplicated list of `(hash, gram_byte_length)`.
pub fn extract_sparse_ngrams_all(data: &[u8]) -> Vec<(NgramHash, usize)> {
    if data.len() < 2 {
        return Vec::new();
    }

    let weights: Vec<u32> = data
        .windows(2)
        .map(|w| qndx_core::pair_weight(w[0], w[1]))
        .collect();

    let n = weights.len(); // number of pair positions (data.len() - 1)
    let mut ngrams = Vec::new();

    for i in 0..n {
        // Every pair position is at minimum a bigram: data[i..i+2]
        let gram = &data[i..i + 2];
        ngrams.push((hash_ngram(gram), gram.len()));

        // Extend rightward looking for qualifying (i, j) pairs
        let mut interior_max: u32 = 0;
        for j in (i + 1)..n {
            // The interior is positions i+1 ..= j-1.
            // When j == i+1 the interior is empty so interior_max stays 0.
            if j > i + 1 {
                interior_max = interior_max.max(weights[j - 1]);
            }

            // Left boundary must exceed interior; if not, no larger j can
            // work either since interior_max only grows.
            if interior_max >= weights[i] {
                break;
            }

            // Right boundary must also exceed interior
            if weights[j] > interior_max {
                let end = j + 2; // pair at j covers bytes j..j+2
                if end <= data.len() {
                    let gram = &data[i..end];
                    ngrams.push((hash_ngram(gram), gram.len()));
                }
            }
        }
    }

    ngrams.sort_unstable();
    ngrams.dedup();
    ngrams
}

/// **`build_covering`** — Extract a minimal covering set of sparse n-grams.
///
/// Uses a monotone-stack algorithm that partitions the input into
/// non-overlapping spans bounded by pair-weight peaks. The result covers
/// every byte position with at least one emitted gram.
///
/// This function is intended for **query-time** use: it produces *fewer*
/// n-grams than trigram extraction for long literals, reducing the number
/// of posting-list lookups required.
///
/// The subset invariant `covering(L) ⊆ all(F)` holds for any string `F`
/// containing literal `L`, provided `F` is indexed with
/// [`extract_sparse_ngrams_all`].
///
/// Returns a sorted, deduplicated list of `(hash, gram_byte_length)`.
pub fn extract_sparse_ngrams_covering(data: &[u8]) -> Vec<(NgramHash, usize)> {
    if data.len() < 2 {
        return Vec::new();
    }

    // Compute weights for each character pair
    let weights: Vec<u32> = data
        .windows(2)
        .map(|w| qndx_core::pair_weight(w[0], w[1]))
        .collect();

    let mut ngrams = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for i in 0..weights.len() {
        while let Some(&top) = stack.last() {
            if weights[top] <= weights[i] {
                // Emit n-gram from top to i
                let start = top;
                let end = i + 2; // +2 because pair at position i covers bytes i..i+2
                if end <= data.len() {
                    let gram = &data[start..end];
                    ngrams.push((hash_ngram(gram), gram.len()));
                }
                if weights[top] == weights[i] {
                    stack.pop();
                    break;
                }
                stack.pop();
            } else {
                break;
            }
        }
        stack.push(i);
    }

    // Drain remaining stack: emit n-grams spanning adjacent stack entries
    while stack.len() > 1 {
        let top = stack.pop().unwrap();
        if let Some(&prev) = stack.last() {
            let end = top + 2;
            if end <= data.len() {
                let gram = &data[prev..end];
                ngrams.push((hash_ngram(gram), gram.len()));
            }
        }
    }

    // If only one pair remains on the stack, emit it as a bigram
    if let Some(&pos) = stack.last() {
        let end = pos + 2;
        if end <= data.len() {
            let gram = &data[pos..end];
            ngrams.push((hash_ngram(gram), gram.len()));
        }
    }

    ngrams.sort_unstable();
    ngrams.dedup();
    ngrams
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn trigram_basic() {
        let trigrams = extract_trigrams(b"abcdef");
        // "abc", "bcd", "cde", "def" -> 4 unique trigrams
        assert_eq!(trigrams.len(), 4);
    }

    #[test]
    fn trigram_short_input() {
        assert!(extract_trigrams(b"ab").is_empty());
        assert_eq!(extract_trigrams(b"abc").len(), 1);
    }

    #[test]
    fn trigram_dedup() {
        // "aaa" has only one unique trigram
        let trigrams = extract_trigrams(b"aaaa");
        assert_eq!(trigrams.len(), 1);
    }

    #[test]
    fn sparse_all_basic() {
        let ngrams = extract_sparse_ngrams_all(b"MAX_FILE_SIZE");
        assert!(!ngrams.is_empty());
    }

    #[test]
    fn sparse_covering_basic() {
        let ngrams = extract_sparse_ngrams_covering(b"MAX_FILE_SIZE");
        assert!(!ngrams.is_empty());
    }

    #[test]
    fn sparse_all_produces_at_least_as_many_as_covering() {
        let input = b"MAX_FILE_SIZE";
        let all: HashSet<u32> = extract_sparse_ngrams_all(input)
            .iter()
            .map(|(h, _)| *h)
            .collect();
        let covering: HashSet<u32> = extract_sparse_ngrams_covering(input)
            .iter()
            .map(|(h, _)| *h)
            .collect();
        assert!(
            covering.is_subset(&all),
            "covering must be a subset of all: covering has {} extra hashes",
            covering.difference(&all).count(),
        );
        assert!(
            all.len() >= covering.len(),
            "all ({}) should have >= covering ({})",
            all.len(),
            covering.len(),
        );
    }

    #[test]
    fn sparse_all_short_input() {
        assert!(extract_sparse_ngrams_all(b"a").is_empty());
        assert!(!extract_sparse_ngrams_all(b"ab").is_empty());
    }

    #[test]
    fn sparse_covering_short_input() {
        assert!(extract_sparse_ngrams_covering(b"a").is_empty());
        assert!(!extract_sparse_ngrams_covering(b"ab").is_empty());
    }

    #[test]
    fn subset_invariant_modified_constant() {
        // The specific case that originally exposed the context-dependency bug.
        let pattern = b"MODIFIED_CONSTANT";
        let content = b"fn main() {\n    let x = MODIFIED_CONSTANT;\n}\n";

        let all: HashSet<u32> = extract_sparse_ngrams_all(content)
            .iter()
            .map(|(h, _)| *h)
            .collect();
        let covering: HashSet<u32> = extract_sparse_ngrams_covering(pattern)
            .iter()
            .map(|(h, _)| *h)
            .collect();

        let missing: Vec<_> = covering.difference(&all).copied().collect();
        assert!(
            missing.is_empty(),
            "covering(pattern) must be ⊆ all(content), missing hashes: {:?}",
            missing,
        );
    }
}
