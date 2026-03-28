//! N-gram extraction: trigram and sparse n-gram algorithms.

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

/// Extract sparse n-grams from a byte slice using the CRC32 pair-weight function.
///
/// A sparse n-gram is a substring where the weights at both ends are strictly
/// greater than all the weights contained inside.
pub fn extract_sparse_ngrams(data: &[u8]) -> Vec<(NgramHash, usize)> {
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
    fn sparse_ngram_basic() {
        let ngrams = extract_sparse_ngrams(b"MAX_FILE_SIZE");
        assert!(!ngrams.is_empty());
    }

    #[test]
    fn sparse_ngram_short_input() {
        assert!(extract_sparse_ngrams(b"a").is_empty());
        assert!(!extract_sparse_ngrams(b"ab").is_empty());
    }
}
