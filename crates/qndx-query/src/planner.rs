//! Query planner: choose optimal n-gram lookup strategy.

use crate::decompose::{decompose_pattern, Decomposition};

/// Plan summary for benchmarking and diagnostics.
#[derive(Debug, Clone)]
pub struct QueryPlan {
    /// The decomposition used.
    pub decomposition: Decomposition,
    /// Number of posting list lookups required.
    pub lookup_count: usize,
    /// Estimated candidate set size (0 = unknown).
    pub estimated_candidates: usize,
}

/// Create a query plan from a regex pattern.
pub fn plan_query(pattern: &str) -> QueryPlan {
    let decomposition = decompose_pattern(pattern);
    let lookup_count = decomposition.required.len()
        + decomposition
            .alternatives
            .iter()
            .map(|a| a.len())
            .sum::<usize>();

    QueryPlan {
        decomposition,
        lookup_count,
        estimated_candidates: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_literal() {
        let plan = plan_query("MAX_FILE_SIZE");
        assert!(plan.lookup_count > 0);
    }

    #[test]
    fn plan_short() {
        let plan = plan_query("ab");
        assert_eq!(plan.lookup_count, 0);
    }
}
