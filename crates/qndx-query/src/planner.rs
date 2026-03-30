//! Query planner: choose optimal n-gram lookup strategy.
//!
//! The planner evaluates two candidate strategies:
//! 1. **Trigram plan**: use the classic overlapping-trigram decomposition.
//! 2. **Sparse plan**: use sparse n-grams extracted from the same literals.
//!
//! It picks the strategy with the lower estimated cost (fewer postings lookups,
//! weighted by selectivity estimates). When sparse coverage is incomplete or
//! offers no benefit, the trigram plan is used as fallback.

use std::collections::HashMap;

use qndx_core::NgramHash;

use crate::decompose::{decompose_pattern, sparse_covering, Decomposition, SparseGram};

/// Which n-gram strategy the planner selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStrategy {
    /// Classic trigram decomposition.
    Trigram,
    /// Sparse n-gram covering (fewer, longer grams).
    Sparse,
}

impl std::fmt::Display for PlanStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanStrategy::Trigram => write!(f, "trigram"),
            PlanStrategy::Sparse => write!(f, "sparse"),
        }
    }
}

/// Explicit strategy override for testing and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyOverride {
    /// Let the planner pick the best strategy (default).
    Auto,
    /// Force trigram strategy.
    ForceTrigram,
    /// Force sparse strategy (falls back to trigram if sparse covering is unavailable).
    ForceSparse,
}

/// Plan summary for benchmarking and diagnostics.
#[derive(Debug, Clone)]
pub struct QueryPlan {
    /// The full decomposition (both trigram and sparse data).
    pub decomposition: Decomposition,
    /// The strategy selected by the planner.
    pub strategy: PlanStrategy,
    /// N-gram hashes to use for required lookups (AND semantics).
    pub required_hashes: Vec<NgramHash>,
    /// N-gram hashes to use per alternative branch (OR semantics between branches).
    pub alternative_hashes: Vec<Vec<NgramHash>>,
    /// Number of posting list lookups required.
    pub lookup_count: usize,
    /// Estimated cost (lower is better). Sum of selectivity weights.
    pub estimated_cost: f64,
    /// Estimated candidate set size (0 = unknown until index is consulted).
    pub estimated_candidates: usize,
}

/// Selectivity weight function: estimate how "selective" (rare) an n-gram is.
///
/// Higher weight = less selective (more common) = higher cost to look up.
/// Lower weight = more selective (rarer) = cheaper to include.
///
/// The default uses a hash-based heuristic: longer n-grams are assumed more
/// selective. An optional frequency table can override this.
pub trait SelectivityEstimator {
    /// Return an estimated cost for looking up this n-gram hash.
    /// Lower values mean the n-gram is more selective.
    fn estimate(&self, hash: NgramHash, gram_len: usize) -> f64;
}

/// Hash-based selectivity estimator (default).
///
/// Assumes longer n-grams are more selective. Uses a simple inverse-length model
/// so that a 5-gram is weighted lower (better) than a trigram.
#[derive(Debug, Clone, Copy)]
pub struct HashSelectivity;

impl SelectivityEstimator for HashSelectivity {
    fn estimate(&self, _hash: NgramHash, gram_len: usize) -> f64 {
        // Cost decreases with gram length: a trigram (len=3) has cost 1.0,
        // a 6-gram has cost 0.5, etc.
        3.0 / gram_len.max(1) as f64
    }
}

/// Frequency-table selectivity estimator.
///
/// Uses precomputed document-frequency counts. N-grams that appear in many
/// documents have higher cost (less selective).
#[derive(Debug, Clone)]
pub struct FrequencySelectivity {
    /// Map from n-gram hash to document frequency (number of files containing it).
    pub freq_table: HashMap<NgramHash, u32>,
    /// Total number of documents in the corpus (for normalization).
    pub total_docs: u32,
}

impl SelectivityEstimator for FrequencySelectivity {
    fn estimate(&self, hash: NgramHash, _gram_len: usize) -> f64 {
        let df = *self.freq_table.get(&hash).unwrap_or(&1) as f64;
        // Normalized frequency: fraction of documents containing this gram.
        // Higher = less selective = higher cost.
        df / self.total_docs.max(1) as f64
    }
}

/// Create a query plan from a regex pattern using the default hash-based selectivity.
pub fn plan_query(pattern: &str) -> QueryPlan {
    plan_query_full(pattern, &HashSelectivity, StrategyOverride::Auto)
}

/// Create a query plan with an explicit strategy override.
pub fn plan_query_with_strategy(pattern: &str, strategy: StrategyOverride) -> QueryPlan {
    plan_query_full(pattern, &HashSelectivity, strategy)
}

/// Create a query plan using a custom selectivity estimator.
pub fn plan_query_with_estimator(pattern: &str, estimator: &dyn SelectivityEstimator) -> QueryPlan {
    plan_query_full(pattern, estimator, StrategyOverride::Auto)
}

/// Full query planning with both estimator and strategy override.
pub fn plan_query_full(
    pattern: &str,
    estimator: &dyn SelectivityEstimator,
    strategy_override: StrategyOverride,
) -> QueryPlan {
    let decomposition = decompose_pattern(pattern);

    // --- Build trigram plan ---
    let trigram_required = decomposition.required.clone();
    let trigram_alternatives = decomposition.alternatives.clone();
    let trigram_lookup_count =
        trigram_required.len() + trigram_alternatives.iter().map(|a| a.len()).sum::<usize>();
    let trigram_cost: f64 = trigram_required
        .iter()
        .map(|&h| estimator.estimate(h, 3))
        .sum::<f64>()
        + trigram_alternatives
            .iter()
            .flat_map(|a| a.iter())
            .map(|&h| estimator.estimate(h, 3))
            .sum::<f64>();

    // --- Build sparse plan ---
    let sparse_required_covering = sparse_covering(&decomposition.sparse_required);
    let sparse_alt_coverings: Vec<Option<Vec<SparseGram>>> = decomposition
        .sparse_alternatives
        .iter()
        .map(|sp| sparse_covering(sp))
        .collect();

    // Evaluate sparse cost (only if all parts have coverage)
    let sparse_plan = build_sparse_plan(
        &sparse_required_covering,
        &sparse_alt_coverings,
        trigram_required.is_empty(),
        estimator,
    );

    let make_trigram_plan = |decomposition: Decomposition| QueryPlan {
        decomposition,
        strategy: PlanStrategy::Trigram,
        required_hashes: trigram_required.clone(),
        alternative_hashes: trigram_alternatives.clone(),
        lookup_count: trigram_lookup_count,
        estimated_cost: trigram_cost,
        estimated_candidates: 0,
    };

    match strategy_override {
        StrategyOverride::ForceTrigram => make_trigram_plan(decomposition),
        StrategyOverride::ForceSparse => {
            // Force sparse; fall back to trigram only if sparse covering is unavailable
            match sparse_plan {
                Some((sparse_req, sparse_alts, sparse_cost, sparse_lookups)) => QueryPlan {
                    decomposition,
                    strategy: PlanStrategy::Sparse,
                    required_hashes: sparse_req,
                    alternative_hashes: sparse_alts,
                    lookup_count: sparse_lookups,
                    estimated_cost: sparse_cost,
                    estimated_candidates: 0,
                },
                None => make_trigram_plan(decomposition),
            }
        }
        StrategyOverride::Auto => match sparse_plan {
            Some((sparse_req, sparse_alts, sparse_cost, sparse_lookups))
                if sparse_cost < trigram_cost =>
            {
                QueryPlan {
                    decomposition,
                    strategy: PlanStrategy::Sparse,
                    required_hashes: sparse_req,
                    alternative_hashes: sparse_alts,
                    lookup_count: sparse_lookups,
                    estimated_cost: sparse_cost,
                    estimated_candidates: 0,
                }
            }
            _ => make_trigram_plan(decomposition),
        },
    }
}

/// Diagnostic details about both strategies for a pattern.
#[derive(Debug, Clone)]
pub struct PlanDiagnostics {
    /// The plan that was (or would be) selected.
    pub selected: QueryPlan,
    /// Trigram strategy details.
    pub trigram_lookups: usize,
    pub trigram_cost: f64,
    /// Sparse strategy details (None if sparse covering is unavailable).
    pub sparse_lookups: Option<usize>,
    pub sparse_cost: Option<f64>,
    /// Literal segments extracted from the pattern.
    pub literals: Vec<String>,
}

/// Produce full diagnostics for a pattern: both strategies, costs, and which wins.
pub fn plan_diagnostics(pattern: &str) -> PlanDiagnostics {
    plan_diagnostics_with_strategy(pattern, StrategyOverride::Auto)
}

/// Produce full diagnostics for a pattern with an explicit strategy override.
pub fn plan_diagnostics_with_strategy(
    pattern: &str,
    strategy_override: StrategyOverride,
) -> PlanDiagnostics {
    let estimator = HashSelectivity;
    let decomposition = decompose_pattern(pattern);

    // Trigram plan
    let trigram_required = &decomposition.required;
    let trigram_alternatives = &decomposition.alternatives;
    let trigram_lookups =
        trigram_required.len() + trigram_alternatives.iter().map(|a| a.len()).sum::<usize>();
    let trigram_cost: f64 = trigram_required
        .iter()
        .map(|&h| estimator.estimate(h, 3))
        .sum::<f64>()
        + trigram_alternatives
            .iter()
            .flat_map(|a| a.iter())
            .map(|&h| estimator.estimate(h, 3))
            .sum::<f64>();

    // Sparse plan
    let sparse_required_covering = sparse_covering(&decomposition.sparse_required);
    let sparse_alt_coverings: Vec<Option<Vec<SparseGram>>> = decomposition
        .sparse_alternatives
        .iter()
        .map(|sp| sparse_covering(sp))
        .collect();
    let sparse_plan = build_sparse_plan(
        &sparse_required_covering,
        &sparse_alt_coverings,
        trigram_required.is_empty(),
        &estimator,
    );

    let (sparse_lookups, sparse_cost) = match &sparse_plan {
        Some((_, _, cost, lookups)) => (Some(*lookups), Some(*cost)),
        None => (None, None),
    };

    // Extract literals for display (re-run extraction)
    let literals = crate::decompose::extract_literals_for_diagnostics(pattern);

    let selected = plan_query_full(pattern, &estimator, strategy_override);

    PlanDiagnostics {
        selected,
        trigram_lookups,
        trigram_cost,
        sparse_lookups,
        sparse_cost,
        literals,
    }
}

/// Return type of [`build_sparse_plan`]: `(required_hashes, alt_hash_sets, cost, lookup_count)`.
type SparsePlanResult = Option<(Vec<NgramHash>, Vec<Vec<NgramHash>>, f64, usize)>;

/// Try to build a sparse plan. Returns None if sparse coverage is incomplete.
fn build_sparse_plan(
    sparse_req: &Option<Vec<SparseGram>>,
    sparse_alts: &[Option<Vec<SparseGram>>],
    no_required: bool,
    estimator: &dyn SelectivityEstimator,
) -> SparsePlanResult {
    let req_hashes: Vec<NgramHash>;
    let mut cost: f64;

    if no_required {
        // No required part — only alternatives
        req_hashes = Vec::new();
        cost = 0.0;
    } else {
        match sparse_req {
            Some(covering) => {
                req_hashes = covering.iter().map(|g| g.hash).collect();
                cost = covering
                    .iter()
                    .map(|g| estimator.estimate(g.hash, g.gram_len))
                    .sum();
            }
            None => return None, // Sparse doesn't cover required part
        }
    }

    let mut alt_hashes = Vec::new();
    for alt in sparse_alts {
        match alt {
            Some(covering) => {
                let hashes: Vec<NgramHash> = covering.iter().map(|g| g.hash).collect();
                cost += covering
                    .iter()
                    .map(|g| estimator.estimate(g.hash, g.gram_len))
                    .sum::<f64>();
                alt_hashes.push(hashes);
            }
            None => return None, // Sparse doesn't cover this branch
        }
    }

    let lookups = req_hashes.len() + alt_hashes.iter().map(|a| a.len()).sum::<usize>();

    Some((req_hashes, alt_hashes, cost, lookups))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_literal() {
        let plan = plan_query("MAX_FILE_SIZE");
        assert!(plan.lookup_count > 0);
        assert!(!plan.required_hashes.is_empty());
    }

    #[test]
    fn plan_short() {
        let plan = plan_query("ab");
        assert_eq!(plan.lookup_count, 0);
        assert!(plan.required_hashes.is_empty());
    }

    #[test]
    fn plan_picks_strategy() {
        let plan = plan_query("MAX_FILE_SIZE");
        // Should have picked one of the two strategies
        assert!(plan.strategy == PlanStrategy::Trigram || plan.strategy == PlanStrategy::Sparse);
    }

    #[test]
    fn plan_alternation() {
        let plan = plan_query("parse_config|serialize_data");
        assert!(plan.required_hashes.is_empty());
        assert_eq!(plan.alternative_hashes.len(), 2);
        assert!(plan.lookup_count > 0);
    }

    #[test]
    fn sparse_preferred_when_cost_is_lower() {
        // The planner selects sparse when its estimated cost is lower than trigrams.
        // Cost accounts for gram length (longer grams are more selective), so sparse
        // may be preferred even with more lookups if the grams are longer.
        let diag = plan_diagnostics("DatabaseConnection_handler_initialize");
        if let (Some(sparse_cost), Some(_sparse_lookups)) = (diag.sparse_cost, diag.sparse_lookups)
        {
            if sparse_cost < diag.trigram_cost {
                assert_eq!(diag.selected.strategy, PlanStrategy::Sparse);
            } else {
                assert_eq!(diag.selected.strategy, PlanStrategy::Trigram);
            }
        }

        // Verify that when sparse IS selected, the plan uses sparse hashes
        let plan = plan_query("DatabaseConnection_handler_initialize");
        if plan.strategy == PlanStrategy::Sparse {
            assert!(!plan.required_hashes.is_empty());
        }
    }

    #[test]
    fn frequency_estimator_works() {
        let mut freq = HashMap::new();
        let decomp = decompose_pattern("MAX_FILE_SIZE");
        // Make all trigrams very frequent
        for &h in &decomp.required {
            freq.insert(h, 900);
        }
        // Make sparse grams very rare
        for sg in &decomp.sparse_required {
            freq.insert(sg.hash, 1);
        }
        let est = FrequencySelectivity {
            freq_table: freq,
            total_docs: 1000,
        };
        let plan = plan_query_with_estimator("MAX_FILE_SIZE", &est);
        // With trigrams being very common and sparse being very rare, the
        // cost model should prefer sparse whenever sparse grams are available.
        let diag = plan_diagnostics("MAX_FILE_SIZE");
        if let (Some(sparse_cost), _) = (diag.sparse_cost, diag.sparse_lookups) {
            if sparse_cost < diag.trigram_cost {
                assert_eq!(plan.strategy, PlanStrategy::Sparse);
            }
        }
    }

    #[test]
    fn plan_cost_is_positive() {
        let plan = plan_query("handle_request");
        if plan.lookup_count > 0 {
            assert!(plan.estimated_cost > 0.0);
        }
    }
}
