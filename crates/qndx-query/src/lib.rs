//! qndx-query: regex decomposition, candidate planner, and verifier.

pub mod decompose;
pub mod planner;
pub mod search;
pub mod verify;

pub use decompose::{sparse_covering, Decomposition, SparseGram};
pub use planner::{
    FrequencySelectivity, HashSelectivity, PlanStrategy, QueryPlan, SelectivityEstimator,
};
pub use search::{
    index_search, index_search_matching_files, index_search_with_overlay, index_search_with_reader,
    IndexSearchResults, IndexSearchStats,
};
