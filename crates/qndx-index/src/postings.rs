//! Postings list representations: Vec<u32>, Roaring, and hybrid.
//!
//! Three representations are supported:
//! - **Vec**: sorted `Vec<u32>` — simple, good for small posting lists.
//! - **Roaring**: compressed bitmap — efficient for large, dense posting lists.
//! - **Hybrid**: automatically picks Vec or Roaring based on a configurable
//!   cardinality threshold (default: 64 entries).
//!
//! Each representation can be serialized to/from bytes for on-disk storage.
//! The on-disk format includes a 1-byte tag so the reader can auto-detect
//! which representation was used at write time.

use std::io::Cursor;

use qndx_core::format::{
    decode_postings, decode_postings_varint, encode_postings, encode_postings_varint,
};
use qndx_core::FileId;
use roaring::RoaringBitmap;

/// Default threshold: posting lists with more than this many entries use Roaring.
pub const DEFAULT_HYBRID_THRESHOLD: usize = 64;

/// On-disk tag bytes for auto-detecting postings format.
const TAG_VEC_FIXED: u8 = 0x01;
const TAG_VEC_VARINT: u8 = 0x02;
const TAG_ROARING: u8 = 0x03;

/// A posting list that can be either a simple sorted vec or a Roaring bitmap.
#[derive(Debug, Clone)]
pub enum PostingList {
    /// Small posting list stored as a sorted Vec<u32>.
    Vec(Vec<FileId>),
    /// Large posting list stored as a Roaring bitmap.
    Roaring(RoaringBitmap),
}

impl PostingList {
    /// Create from a sorted vec, choosing representation based on size.
    /// Uses the default hybrid threshold.
    pub fn from_vec(ids: Vec<FileId>) -> Self {
        Self::from_vec_with_threshold(ids, DEFAULT_HYBRID_THRESHOLD)
    }

    /// Create from a sorted vec with a custom hybrid threshold.
    pub fn from_vec_with_threshold(mut ids: Vec<FileId>, threshold: usize) -> Self {
        ids.sort_unstable();
        ids.dedup();
        if ids.len() > threshold {
            let mut bitmap = RoaringBitmap::new();
            for &id in &ids {
                bitmap.insert(id);
            }
            PostingList::Roaring(bitmap)
        } else {
            PostingList::Vec(ids)
        }
    }

    /// Force creation as a Vec regardless of size (for benchmarking).
    pub fn force_vec(mut ids: Vec<FileId>) -> Self {
        ids.sort_unstable();
        ids.dedup();
        PostingList::Vec(ids)
    }

    /// Force creation as a Roaring bitmap regardless of size (for benchmarking).
    pub fn force_roaring(ids: &[FileId]) -> Self {
        let mut bitmap = RoaringBitmap::new();
        for &id in ids {
            bitmap.insert(id);
        }
        PostingList::Roaring(bitmap)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        match self {
            PostingList::Vec(v) => v.len(),
            PostingList::Roaring(r) => r.len() as usize,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns true if this posting list uses the Vec representation.
    pub fn is_vec(&self) -> bool {
        matches!(self, PostingList::Vec(_))
    }

    /// Returns true if this posting list uses the Roaring representation.
    pub fn is_roaring(&self) -> bool {
        matches!(self, PostingList::Roaring(_))
    }

    /// Intersect two posting lists.
    pub fn intersect(&self, other: &PostingList) -> PostingList {
        match (self, other) {
            (PostingList::Vec(a), PostingList::Vec(b)) => PostingList::Vec(vec_intersect(a, b)),
            (PostingList::Roaring(a), PostingList::Roaring(b)) => PostingList::Roaring(a & b),
            (PostingList::Vec(v), PostingList::Roaring(r))
            | (PostingList::Roaring(r), PostingList::Vec(v)) => {
                let result: Vec<FileId> = v.iter().copied().filter(|id| r.contains(*id)).collect();
                PostingList::from_vec(result)
            }
        }
    }

    /// Union two posting lists.
    pub fn union(&self, other: &PostingList) -> PostingList {
        match (self, other) {
            (PostingList::Vec(a), PostingList::Vec(b)) => PostingList::from_vec(vec_union(a, b)),
            (PostingList::Roaring(a), PostingList::Roaring(b)) => PostingList::Roaring(a | b),
            (PostingList::Vec(v), PostingList::Roaring(r))
            | (PostingList::Roaring(r), PostingList::Vec(v)) => {
                let mut result = r.clone();
                for &id in v {
                    result.insert(id);
                }
                PostingList::Roaring(result)
            }
        }
    }

    /// Convert to a sorted vec of FileIds.
    pub fn to_vec(&self) -> Vec<FileId> {
        match self {
            PostingList::Vec(v) => v.clone(),
            PostingList::Roaring(r) => r.iter().collect(),
        }
    }

    // -----------------------------------------------------------------------
    // Serialization: encode to bytes with a tag prefix for auto-detection
    // -----------------------------------------------------------------------

    /// Encode using fixed-width delta encoding (tag 0x01).
    pub fn encode_fixed(&self) -> Vec<u8> {
        let ids = self.to_vec();
        let mut buf = Vec::with_capacity(1 + 4 + ids.len() * 4);
        buf.push(TAG_VEC_FIXED);
        buf.extend_from_slice(&encode_postings(&ids));
        buf
    }

    /// Encode using varint delta encoding (tag 0x02).
    pub fn encode_varint(&self) -> Vec<u8> {
        let ids = self.to_vec();
        let mut buf = Vec::with_capacity(1 + 5 + ids.len() * 3);
        buf.push(TAG_VEC_VARINT);
        buf.extend_from_slice(&encode_postings_varint(&ids));
        buf
    }

    /// Encode using Roaring bitmap serialization (tag 0x03).
    pub fn encode_roaring(&self) -> Vec<u8> {
        let bitmap = match self {
            PostingList::Roaring(r) => r.clone(),
            PostingList::Vec(v) => {
                let mut b = RoaringBitmap::new();
                for &id in v {
                    b.insert(id);
                }
                b
            }
        };
        let serialized_len = bitmap.serialized_size();
        let mut buf = Vec::with_capacity(1 + serialized_len);
        buf.push(TAG_ROARING);
        bitmap.serialize_into(&mut buf).expect("roaring serialize");
        buf
    }

    /// Encode using the optimal format for this posting list:
    /// - Vec lists: varint delta encoding (compact)
    /// - Roaring lists: native Roaring serialization
    pub fn encode_auto(&self) -> Vec<u8> {
        match self {
            PostingList::Vec(_) => self.encode_varint(),
            PostingList::Roaring(_) => self.encode_roaring(),
        }
    }

    /// Decode a posting list from tagged bytes (auto-detects format).
    pub fn decode_tagged(data: &[u8]) -> Option<Self> {
        if data.is_empty() {
            return None;
        }
        let tag = data[0];
        let payload = &data[1..];
        match tag {
            TAG_VEC_FIXED => {
                let ids = decode_postings(payload);
                Some(PostingList::Vec(ids))
            }
            TAG_VEC_VARINT => {
                let ids = decode_postings_varint(payload);
                Some(PostingList::Vec(ids))
            }
            TAG_ROARING => {
                let bitmap = RoaringBitmap::deserialize_from(Cursor::new(payload)).ok()?;
                Some(PostingList::Roaring(bitmap))
            }
            _ => None,
        }
    }

    /// Encoded size using varint delta encoding (without allocating).
    pub fn varint_encoded_size(&self) -> usize {
        1 + qndx_core::format::varint_encoded_size(&self.to_vec())
    }

    /// Encoded size using Roaring serialization (without full serialize).
    pub fn roaring_encoded_size(&self) -> usize {
        match self {
            PostingList::Roaring(r) => 1 + r.serialized_size(),
            PostingList::Vec(v) => {
                let mut b = RoaringBitmap::new();
                for &id in v {
                    b.insert(id);
                }
                1 + b.serialized_size()
            }
        }
    }
}

fn vec_intersect(a: &[FileId], b: &[FileId]) -> Vec<FileId> {
    let mut result = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                result.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    result
}

fn vec_union(a: &[FileId], b: &[FileId]) -> Vec<FileId> {
    let mut result = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                result.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                result.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                result.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    result.extend_from_slice(&a[i..]);
    result.extend_from_slice(&b[j..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_list_uses_vec() {
        let pl = PostingList::from_vec((0..10).collect());
        assert!(pl.is_vec());
    }

    #[test]
    fn large_list_uses_roaring() {
        let pl = PostingList::from_vec((0..100).collect());
        assert!(pl.is_roaring());
    }

    #[test]
    fn custom_threshold() {
        // With threshold=10, 20 items should be Roaring
        let pl = PostingList::from_vec_with_threshold((0..20).collect(), 10);
        assert!(pl.is_roaring());

        // With threshold=200, 100 items should stay Vec
        let pl = PostingList::from_vec_with_threshold((0..100).collect(), 200);
        assert!(pl.is_vec());
    }

    #[test]
    fn force_vec() {
        let pl = PostingList::force_vec((0..1000).collect());
        assert!(pl.is_vec());
        assert_eq!(pl.len(), 1000);
    }

    #[test]
    fn force_roaring() {
        let pl = PostingList::force_roaring(&[1, 2, 3]);
        assert!(pl.is_roaring());
        assert_eq!(pl.len(), 3);
    }

    #[test]
    fn intersect_vec_vec() {
        let a = PostingList::from_vec(vec![1, 2, 3, 4, 5]);
        let b = PostingList::from_vec(vec![3, 4, 5, 6, 7]);
        let result = a.intersect(&b).to_vec();
        assert_eq!(result, vec![3, 4, 5]);
    }

    #[test]
    fn intersect_roaring_roaring() {
        let a = PostingList::force_roaring(&[1, 2, 3, 4, 5]);
        let b = PostingList::force_roaring(&[3, 4, 5, 6, 7]);
        let result = a.intersect(&b).to_vec();
        assert_eq!(result, vec![3, 4, 5]);
    }

    #[test]
    fn intersect_mixed() {
        let a = PostingList::force_vec(vec![1, 2, 3, 4, 5]);
        let b = PostingList::force_roaring(&[3, 4, 5, 6, 7]);
        let result = a.intersect(&b).to_vec();
        assert_eq!(result, vec![3, 4, 5]);
    }

    #[test]
    fn union_vec_vec() {
        let a = PostingList::from_vec(vec![1, 3, 5]);
        let b = PostingList::from_vec(vec![2, 3, 4]);
        let result = a.union(&b).to_vec();
        assert_eq!(result, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn union_roaring_roaring() {
        let a = PostingList::force_roaring(&[1, 3, 5]);
        let b = PostingList::force_roaring(&[2, 3, 4]);
        let result = a.union(&b).to_vec();
        assert_eq!(result, vec![1, 2, 3, 4, 5]);
    }

    // --- Serialization roundtrip tests ---

    #[test]
    fn encode_decode_fixed_roundtrip() {
        let pl = PostingList::from_vec(vec![1, 5, 10, 20, 100]);
        let encoded = pl.encode_fixed();
        assert_eq!(encoded[0], TAG_VEC_FIXED);
        let decoded = PostingList::decode_tagged(&encoded).unwrap();
        assert_eq!(decoded.to_vec(), vec![1, 5, 10, 20, 100]);
    }

    #[test]
    fn encode_decode_varint_roundtrip() {
        let pl = PostingList::from_vec(vec![1, 5, 10, 20, 100]);
        let encoded = pl.encode_varint();
        assert_eq!(encoded[0], TAG_VEC_VARINT);
        let decoded = PostingList::decode_tagged(&encoded).unwrap();
        assert_eq!(decoded.to_vec(), vec![1, 5, 10, 20, 100]);
    }

    #[test]
    fn encode_decode_roaring_roundtrip() {
        let ids: Vec<u32> = (0..200).collect();
        let pl = PostingList::force_roaring(&ids);
        let encoded = pl.encode_roaring();
        assert_eq!(encoded[0], TAG_ROARING);
        let decoded = PostingList::decode_tagged(&encoded).unwrap();
        assert_eq!(decoded.to_vec(), ids);
    }

    #[test]
    fn encode_auto_vec_uses_varint() {
        let pl = PostingList::force_vec(vec![1, 5, 10]);
        let encoded = pl.encode_auto();
        assert_eq!(encoded[0], TAG_VEC_VARINT);
    }

    #[test]
    fn encode_auto_roaring_uses_roaring() {
        let pl = PostingList::force_roaring(&[1, 5, 10]);
        let encoded = pl.encode_auto();
        assert_eq!(encoded[0], TAG_ROARING);
    }

    #[test]
    fn decode_tagged_empty_returns_none() {
        assert!(PostingList::decode_tagged(&[]).is_none());
    }

    #[test]
    fn decode_tagged_unknown_tag_returns_none() {
        assert!(PostingList::decode_tagged(&[0xFF, 0x00]).is_none());
    }

    #[test]
    fn varint_encoding_smaller_than_fixed() {
        let ids: Vec<u32> = (0..50).collect();
        let pl = PostingList::force_vec(ids);
        let fixed = pl.encode_fixed();
        let varint = pl.encode_varint();
        assert!(
            varint.len() < fixed.len(),
            "varint ({}) should be smaller than fixed ({})",
            varint.len(),
            fixed.len()
        );
    }

    #[test]
    fn roaring_large_roundtrip() {
        let ids: Vec<u32> = (0..10_000).step_by(3).collect();
        let pl = PostingList::force_roaring(&ids);
        let encoded = pl.encode_roaring();
        let decoded = PostingList::decode_tagged(&encoded).unwrap();
        assert_eq!(decoded.to_vec(), ids);
    }
}
