//! Property tests for trigram decomposition and postings operations (Issue #24).
//!
//! Uses proptest for randomized input coverage.

use proptest::prelude::*;
use qndx_core::format::{decode_postings, encode_postings};
use qndx_core::FileId;
use qndx_index::ngram::extract_trigrams;
use qndx_index::postings::PostingList;

// ---------------------------------------------------------------------------
// Trigram decomposition properties
// ---------------------------------------------------------------------------

proptest! {
    /// Decomposition always produces valid trigrams (hashes) from any input.
    #[test]
    fn trigram_extraction_never_panics(data in prop::collection::vec(any::<u8>(), 0..1000)) {
        let _ = extract_trigrams(&data);
    }

    /// Trigrams from a string with length >= 3 always produces at least one trigram.
    #[test]
    fn trigram_nonempty_for_sufficient_input(data in prop::collection::vec(any::<u8>(), 3..200)) {
        let trigrams = extract_trigrams(&data);
        prop_assert!(!trigrams.is_empty(), "input of len {} should produce trigrams", data.len());
    }

    /// Trigram output is always sorted and deduplicated.
    #[test]
    fn trigrams_are_sorted_and_deduped(data in prop::collection::vec(any::<u8>(), 0..500)) {
        let trigrams = extract_trigrams(&data);
        // Sorted
        for w in trigrams.windows(2) {
            prop_assert!(w[0] <= w[1], "not sorted: {} > {}", w[0], w[1]);
        }
        // Deduplicated (no adjacent duplicates since sorted)
        for w in trigrams.windows(2) {
            prop_assert!(w[0] != w[1], "duplicate found: {}", w[0]);
        }
    }

    /// Roundtrip: trigrams extracted from a string, when looked up, should match
    /// that string. We verify that the hash of each 3-byte window appears in the output.
    #[test]
    fn trigram_roundtrip_all_windows_present(data in prop::collection::vec(any::<u8>(), 3..100)) {
        let trigrams = extract_trigrams(&data);
        for window in data.windows(3) {
            let hash = qndx_core::hash_ngram(window);
            prop_assert!(
                trigrams.contains(&hash),
                "hash of {:?} not found in trigrams",
                window,
            );
        }
    }

    /// Short inputs (< 3 bytes) produce no trigrams.
    #[test]
    fn trigram_short_input_empty(data in prop::collection::vec(any::<u8>(), 0..3)) {
        let trigrams = extract_trigrams(&data);
        prop_assert!(trigrams.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Postings operations properties
// ---------------------------------------------------------------------------

/// Strategy for generating sorted, deduplicated FileId vectors.
fn sorted_file_ids(max_len: usize) -> impl Strategy<Value = Vec<FileId>> {
    prop::collection::vec(0u32..10_000, 0..max_len).prop_map(|mut v| {
        v.sort_unstable();
        v.dedup();
        v
    })
}

proptest! {
    /// Intersection is commutative: A ∩ B == B ∩ A
    #[test]
    fn intersection_commutative(
        a in sorted_file_ids(100),
        b in sorted_file_ids(100),
    ) {
        let pa = PostingList::from_vec(a);
        let pb = PostingList::from_vec(b);
        let ab = pa.intersect(&pb).to_vec();
        let ba = pb.intersect(&pa).to_vec();
        prop_assert_eq!(ab, ba);
    }

    /// Union is commutative: A ∪ B == B ∪ A
    #[test]
    fn union_commutative(
        a in sorted_file_ids(100),
        b in sorted_file_ids(100),
    ) {
        let pa = PostingList::from_vec(a);
        let pb = PostingList::from_vec(b);
        let ab = pa.union(&pb).to_vec();
        let ba = pb.union(&pa).to_vec();
        prop_assert_eq!(ab, ba);
    }

    /// Intersection is associative: (A ∩ B) ∩ C == A ∩ (B ∩ C)
    #[test]
    fn intersection_associative(
        a in sorted_file_ids(50),
        b in sorted_file_ids(50),
        c in sorted_file_ids(50),
    ) {
        let pa = PostingList::from_vec(a);
        let pb = PostingList::from_vec(b);
        let pc = PostingList::from_vec(c);
        let ab_c = pa.intersect(&pb).intersect(&pc).to_vec();
        let a_bc = pa.intersect(&pb.intersect(&pc)).to_vec();
        prop_assert_eq!(ab_c, a_bc);
    }

    /// Union is associative: (A ∪ B) ∪ C == A ∪ (B ∪ C)
    #[test]
    fn union_associative(
        a in sorted_file_ids(50),
        b in sorted_file_ids(50),
        c in sorted_file_ids(50),
    ) {
        let pa = PostingList::from_vec(a);
        let pb = PostingList::from_vec(b);
        let pc = PostingList::from_vec(c);
        let ab_c = pa.union(&pb).union(&pc).to_vec();
        let a_bc = pa.union(&pb.union(&pc)).to_vec();
        prop_assert_eq!(ab_c, a_bc);
    }

    /// Intersection result is a subset of both inputs.
    #[test]
    fn intersection_is_subset(
        a in sorted_file_ids(100),
        b in sorted_file_ids(100),
    ) {
        let pa = PostingList::from_vec(a.clone());
        let pb = PostingList::from_vec(b.clone());
        let result = pa.intersect(&pb).to_vec();

        for &id in &result {
            prop_assert!(a.contains(&id), "intersection result {} not in A", id);
            prop_assert!(b.contains(&id), "intersection result {} not in B", id);
        }
    }

    /// Union result is a superset of both inputs.
    #[test]
    fn union_is_superset(
        a in sorted_file_ids(100),
        b in sorted_file_ids(100),
    ) {
        let pa = PostingList::from_vec(a.clone());
        let pb = PostingList::from_vec(b.clone());
        let result = pa.union(&pb).to_vec();

        for &id in &a {
            prop_assert!(result.contains(&id), "A element {} missing from union", id);
        }
        for &id in &b {
            prop_assert!(result.contains(&id), "B element {} missing from union", id);
        }
    }

    /// Intersection with self is identity: A ∩ A == A
    #[test]
    fn intersection_with_self(a in sorted_file_ids(100)) {
        let pa = PostingList::from_vec(a.clone());
        let pa2 = PostingList::from_vec(a.clone());
        let result = pa.intersect(&pa2).to_vec();
        prop_assert_eq!(result, a);
    }

    /// Union with self is identity: A ∪ A == A
    #[test]
    fn union_with_self(a in sorted_file_ids(100)) {
        let pa = PostingList::from_vec(a.clone());
        let pa2 = PostingList::from_vec(a.clone());
        let result = pa.union(&pa2).to_vec();
        prop_assert_eq!(result, a);
    }

    /// Intersection with empty is empty.
    #[test]
    fn intersection_with_empty(a in sorted_file_ids(100)) {
        let pa = PostingList::from_vec(a);
        let empty = PostingList::from_vec(vec![]);
        let result = pa.intersect(&empty).to_vec();
        prop_assert!(result.is_empty());
    }

    /// Union with empty is identity.
    #[test]
    fn union_with_empty(a in sorted_file_ids(100)) {
        let pa = PostingList::from_vec(a.clone());
        let empty = PostingList::from_vec(vec![]);
        let result = pa.union(&empty).to_vec();
        prop_assert_eq!(result, a);
    }
}

// ---------------------------------------------------------------------------
// Postings encode/decode roundtrip
// ---------------------------------------------------------------------------

proptest! {
    /// Encode/decode roundtrip for postings lists.
    #[test]
    fn postings_encode_decode_roundtrip(ids in sorted_file_ids(200)) {
        let encoded = encode_postings(&ids);
        let decoded = decode_postings(&encoded);
        prop_assert_eq!(decoded, ids);
    }

    /// Empty postings encode/decode correctly.
    #[test]
    fn postings_encode_decode_empty(_dummy in 0u8..1) {
        let ids: Vec<FileId> = vec![];
        let encoded = encode_postings(&ids);
        let decoded = decode_postings(&encoded);
        prop_assert_eq!(decoded, ids);
    }

    /// Single-element postings encode/decode correctly.
    #[test]
    fn postings_encode_decode_single(id in 0u32..100_000) {
        let ids = vec![id];
        let encoded = encode_postings(&ids);
        let decoded = decode_postings(&encoded);
        prop_assert_eq!(decoded, ids);
    }
}
