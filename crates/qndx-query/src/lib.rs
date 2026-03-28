//! qndx-query: regex decomposition, candidate planner, and verifier.

pub mod decompose;
pub mod planner;
pub mod verify;
pub mod search;

pub use search::{index_search, index_search_matching_files, index_search_with_reader, IndexSearchResults, IndexSearchStats};
