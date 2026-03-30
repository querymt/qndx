//! qndx-query: regex decomposition, candidate planner, and verifier.

pub mod decompose;
pub mod planner;
pub mod search;
pub mod verify;

pub use decompose::{Decomposition, SparseGram, sparse_covering};
pub use planner::{
    FrequencySelectivity, HashSelectivity, PlanDiagnostics, PlanStrategy, QueryPlan,
    SelectivityEstimator, StrategyOverride, plan_diagnostics, plan_diagnostics_with_strategy,
    plan_query_with_strategy,
};
pub use search::{
    IndexSearchResults, IndexSearchStats, index_search, index_search_matching_files,
    index_search_with_overlay, index_search_with_overlay_and_timing, index_search_with_reader,
    index_search_with_strategy, index_search_with_strategy_and_timing,
};
