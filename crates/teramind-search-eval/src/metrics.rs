//! Ranking-metric primitives.
//!
//! All functions take `relevance: &[u32]` where each entry is the graded
//! relevance (0 = irrelevant, 1 = relevant, 2 = strongly relevant) of
//! the i-th *ranked* hit. Index 0 is the top hit.

/// Discounted cumulative gain at rank `k`.
///
/// DCG@K = sum (rel_i / log2(i + 2)) for i in 0..min(K, ranked.len())
pub fn dcg_at_k(relevance: &[u32], k: usize) -> f64 {
    relevance
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &r)| (r as f64) / ((i as f64 + 2.0).log2()))
        .sum()
}

/// Ideal DCG@K — DCG of the ranking sorted descending by relevance.
pub fn idcg_at_k(relevance: &[u32], k: usize) -> f64 {
    let mut sorted: Vec<u32> = relevance.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    dcg_at_k(&sorted, k)
}

/// Normalized DCG@K: nDCG = DCG / IDCG.
///
/// Returns 0.0 when there are no relevant items (IDCG = 0).
pub fn ndcg_at_k(relevance: &[u32], k: usize) -> f64 {
    let i = idcg_at_k(relevance, k);
    if i == 0.0 { return 0.0; }
    (dcg_at_k(relevance, k) / i).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dcg_known_answer() {
        // Relevance [3, 2, 3, 0, 1, 2] -> DCG@6 ~ 6.86 (classic textbook example).
        let r: Vec<u32> = vec![3, 2, 3, 0, 1, 2];
        let dcg = dcg_at_k(&r, 6);
        assert!((dcg - 6.8611).abs() < 0.001, "got {dcg}");
    }

    #[test]
    fn ndcg_of_ideal_ranking_is_one() {
        let r: Vec<u32> = vec![2, 2, 1, 1, 0];
        assert!((ndcg_at_k(&r, 5) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ndcg_of_reversed_ranking_is_less_than_one() {
        let r: Vec<u32> = vec![0, 1, 1, 2, 2];
        assert!(ndcg_at_k(&r, 5) < 1.0);
    }

    #[test]
    fn ndcg_zero_when_no_relevant_hits() {
        let r: Vec<u32> = vec![0, 0, 0];
        assert_eq!(ndcg_at_k(&r, 3), 0.0);
    }

    #[test]
    fn ndcg_handles_short_lists() {
        let r: Vec<u32> = vec![2];
        assert!((ndcg_at_k(&r, 10) - 1.0).abs() < 1e-9);
    }
}
