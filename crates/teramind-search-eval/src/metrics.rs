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
    if i == 0.0 {
        return 0.0;
    }
    (dcg_at_k(relevance, k) / i).clamp(0.0, 1.0)
}

/// Mean Reciprocal Rank for a single ranked list: 1 / rank of the first
/// hit with relevance > 0. Returns 0.0 when no relevant hit is found.
pub fn mrr(relevance: &[u32]) -> f64 {
    relevance
        .iter()
        .enumerate()
        .find(|(_, &r)| r > 0)
        .map(|(i, _)| 1.0 / (i as f64 + 1.0))
        .unwrap_or(0.0)
}

/// Precision@K: fraction of the top-K hits that are relevant.
/// Denominator is `k` (not `min(k, len)`) — matches standard IR convention.
pub fn precision_at_k(relevance: &[u32], k: usize) -> f64 {
    if k == 0 {
        return 0.0;
    }
    let hit = relevance.iter().take(k).filter(|&&r| r > 0).count();
    hit as f64 / k as f64
}

/// Recall@K: fraction of all relevant items in the corpus that appear
/// in the top-K hits. Returns 0.0 when `total_relevant == 0`.
pub fn recall_at_k(relevance: &[u32], k: usize, total_relevant: u32) -> f64 {
    if total_relevant == 0 {
        return 0.0;
    }
    let hit = relevance.iter().take(k).filter(|&&r| r > 0).count();
    hit as f64 / total_relevant as f64
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

    #[test]
    fn mrr_takes_reciprocal_rank_of_first_relevant() {
        // Relevance > 0 starts at index 2 (rank 3) -> MRR = 1/3.
        let r: Vec<u32> = vec![0, 0, 2, 1];
        assert!((mrr(&r) - (1.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn mrr_zero_when_no_relevant() {
        let r: Vec<u32> = vec![0, 0, 0];
        assert_eq!(mrr(&r), 0.0);
    }

    #[test]
    fn precision_at_k_counts_relevant_in_top_k() {
        let r: Vec<u32> = vec![1, 0, 2, 0, 1];
        assert!((precision_at_k(&r, 5) - 0.6).abs() < 1e-9);
        assert!((precision_at_k(&r, 3) - (2.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn precision_at_k_short_list_uses_actual_len() {
        // When k > len, the denominator is still k -> matches IR convention.
        let r: Vec<u32> = vec![1, 1];
        assert!((precision_at_k(&r, 5) - 0.4).abs() < 1e-9);
    }

    #[test]
    fn recall_at_k_uses_total_relevant_count() {
        // total_relevant = 3, top-K hits = 2 relevant -> 2/3.
        let r: Vec<u32> = vec![1, 0, 1, 0];
        assert!((recall_at_k(&r, 4, 3) - (2.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn recall_at_k_zero_when_no_relevant_in_corpus() {
        let r: Vec<u32> = vec![0, 0];
        assert_eq!(recall_at_k(&r, 2, 0), 0.0);
    }

    use proptest::prelude::*;
    proptest! {
        #[test]
        fn ndcg_always_between_zero_and_one(
            rels in proptest::collection::vec(0u32..=2u32, 0..50),
            k    in 1usize..50usize,
        ) {
            let n = ndcg_at_k(&rels, k);
            prop_assert!((0.0..=1.0).contains(&n), "nDCG out of [0,1]: {n}");
        }
    }
}
