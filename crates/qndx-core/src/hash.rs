//! Hashing utilities for n-grams and file identification.

use crc32fast::Hasher;

/// Hash a byte slice using CRC32 (used for n-gram hashing).
pub fn hash_ngram(bytes: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    hasher.finalize()
}

/// Compute the weight of a character pair for sparse n-gram extraction.
/// Uses CRC32 as the default deterministic weight function.
pub fn pair_weight(a: u8, b: u8) -> u32 {
    hash_ngram(&[a, b])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        let h1 = hash_ngram(b"abc");
        let h2 = hash_ngram(b"abc");
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_inputs_differ() {
        let h1 = hash_ngram(b"abc");
        let h2 = hash_ngram(b"abd");
        assert_ne!(h1, h2);
    }

    #[test]
    fn pair_weight_deterministic() {
        let w1 = pair_weight(b'a', b'b');
        let w2 = pair_weight(b'a', b'b');
        assert_eq!(w1, w2);
    }
}
