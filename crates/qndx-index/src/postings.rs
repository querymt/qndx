//! Postings list representations: Vec<u32>, Roaring, and hybrid.

use qndx_core::FileId;
use roaring::RoaringBitmap;

/// Threshold: posting lists with more than this many entries use Roaring.
const HYBRID_THRESHOLD: usize = 64;

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
    pub fn from_vec(mut ids: Vec<FileId>) -> Self {
        ids.sort_unstable();
        ids.dedup();
        if ids.len() > HYBRID_THRESHOLD {
            let mut bitmap = RoaringBitmap::new();
            for &id in &ids {
                bitmap.insert(id);
            }
            PostingList::Roaring(bitmap)
        } else {
            PostingList::Vec(ids)
        }
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

    /// Intersect two posting lists.
    pub fn intersect(&self, other: &PostingList) -> PostingList {
        match (self, other) {
            (PostingList::Vec(a), PostingList::Vec(b)) => {
                PostingList::Vec(vec_intersect(a, b))
            }
            (PostingList::Roaring(a), PostingList::Roaring(b)) => {
                PostingList::Roaring(a & b)
            }
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
            (PostingList::Vec(a), PostingList::Vec(b)) => {
                PostingList::from_vec(vec_union(a, b))
            }
            (PostingList::Roaring(a), PostingList::Roaring(b)) => {
                PostingList::Roaring(a | b)
            }
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
        assert!(matches!(pl, PostingList::Vec(_)));
    }

    #[test]
    fn large_list_uses_roaring() {
        let pl = PostingList::from_vec((0..100).collect());
        assert!(matches!(pl, PostingList::Roaring(_)));
    }

    #[test]
    fn intersect_vec_vec() {
        let a = PostingList::from_vec(vec![1, 2, 3, 4, 5]);
        let b = PostingList::from_vec(vec![3, 4, 5, 6, 7]);
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
}
