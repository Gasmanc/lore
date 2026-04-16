//! Mean Reciprocal Rank computation.

/// Computes MRR from a slice of reciprocal rank values.
///
/// Each element should be `1/rank` for the first relevant result, or `0.0`
/// if no relevant result was found within the cutoff.  Returns `0.0` for an
/// empty slice.
#[allow(clippy::cast_precision_loss)]
pub fn compute(reciprocal_ranks: &[f64]) -> f64 {
    if reciprocal_ranks.is_empty() {
        return 0.0;
    }
    reciprocal_ranks.iter().sum::<f64>() / reciprocal_ranks.len() as f64
}

/// Returns the reciprocal rank for a single result list.
///
/// Searches `results` for the first entry whose path ends with
/// `expected_doc`.  Returns `1/rank` (1-indexed) or `0.0` if not found
/// within the first `cutoff` results.
#[allow(clippy::cast_precision_loss)]
pub fn reciprocal_rank<T: AsRef<str>>(results: &[T], expected_doc: &str, cutoff: usize) -> f64 {
    results
        .iter()
        .take(cutoff)
        .position(|r| r.as_ref().ends_with(expected_doc))
        .map_or(0.0, |i| 1.0 / (i + 1) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mrr_all_rank_one() {
        assert!((compute(&[1.0, 1.0, 1.0]) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn mrr_empty() {
        assert!((compute(&[])).abs() < f64::EPSILON);
    }

    #[test]
    fn mrr_mixed() {
        // RR: 1/1, 1/2, 0 → mean = (1 + 0.5 + 0) / 3 ≈ 0.5
        let rrs = [1.0, 0.5, 0.0];
        let got = compute(&rrs);
        assert!((got - 0.5).abs() < 1e-10, "expected 0.5, got {got}");
    }

    #[test]
    fn reciprocal_rank_found_at_first() {
        let results = vec!["foo.md", "bar.md", "baz.md"];
        assert!((reciprocal_rank(&results, "foo.md", 10) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn reciprocal_rank_found_at_third() {
        let results = vec!["a.md", "b.md", "target.md"];
        let rr = reciprocal_rank(&results, "target.md", 10);
        assert!((rr - 1.0 / 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn reciprocal_rank_not_found() {
        let results = vec!["a.md", "b.md"];
        assert!((reciprocal_rank(&results, "missing.md", 10)).abs() < f64::EPSILON);
    }

    #[test]
    fn reciprocal_rank_beyond_cutoff() {
        let results = vec!["a.md", "b.md", "target.md", "c.md"];
        // cutoff=2 means only first 2 are checked
        assert!((reciprocal_rank(&results, "target.md", 2)).abs() < f64::EPSILON);
    }
}
