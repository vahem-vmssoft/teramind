use teramind_core::types::Hit;
use teramind_core::ids::{FileDiffId, SessionId, SkillId, ToolCallId, TurnId};
use teramind_db::repos::search::{RankedDiff, RankedSkill, RankedTurn};
use std::time::Instant;
use time::OffsetDateTime;
use uuid::Uuid;

/// Configuration for the ranking blend. Spec §6.1 default weights.
#[derive(Debug, Clone, Copy)]
pub struct BlendWeights {
    pub fts: f32,
    pub trgm: f32,
    pub recency: f32,
    pub project: f32,
}

impl Default for BlendWeights {
    fn default() -> Self {
        Self { fts: 0.6, trgm: 0.4, recency: 0.2, project: 0.3 }
    }
}

pub fn final_score(fts: f32, trgm: f32, ts: OffsetDateTime, weights: BlendWeights, same_project: bool) -> f32 {
    let recency_decay = recency_factor(ts);
    let project_boost = if same_project { 1.0 } else { 0.0 };
    weights.fts * fts
        + weights.trgm * trgm
        + weights.recency * recency_decay
        + weights.project * project_boost
}

fn recency_factor(ts: OffsetDateTime) -> f32 {
    let age = OffsetDateTime::now_utc() - ts;
    let days = age.whole_seconds() as f32 / 86_400.0;
    (-days / 90.0).exp()
}

pub fn rank_and_hydrate(
    fts_turns: Vec<RankedTurn>,
    trgm_diffs: Vec<RankedDiff>,
    trgm_skills: Vec<RankedSkill>,
    weights: BlendWeights,
    same_project_id: Option<Uuid>,
    limit: u32,
) -> Vec<Hit> {
    use std::collections::HashMap;
    let mut by_turn: HashMap<Uuid, RankedTurn> = HashMap::new();
    for t in fts_turns.into_iter() {
        by_turn.insert(t.turn_id, t);
    }
    let mut hits: Vec<(f32, Hit)> = Vec::new();
    for t in by_turn.into_values() {
        let same_p = same_project_id.map(|p| Some(p) == t.project_id).unwrap_or(false);
        let score = final_score(t.fts_score, t.trgm_score, t.ts, weights, same_p);
        let snippet = build_snippet(&t.user_prompt, &t.assistant_text);
        hits.push((score, Hit::Turn {
            turn_id: TurnId(t.turn_id),
            session_id: SessionId(t.session_id),
            ordinal: t.ordinal,
            snippet,
            score,
            ts: t.ts,
        }));
    }
    for d in trgm_diffs {
        let same_p = same_project_id.map(|p| Some(p) == d.project_id).unwrap_or(false);
        let score = final_score(0.0, d.trgm_score, d.ts, weights, same_p);
        let snippet = if d.post_excerpt.is_empty() { d.pre_excerpt.clone() } else { d.post_excerpt.clone() };
        hits.push((score, Hit::FileDiff {
            diff_id: FileDiffId(d.diff_id),
            rel_path: d.rel_path,
            hunk_snippet: snippet,
            score,
            ts: d.ts,
        }));
    }
    for s in trgm_skills {
        let score = final_score(0.0, s.trgm_score, OffsetDateTime::now_utc(), weights, false);
        hits.push((score, Hit::Skill {
            skill_id: SkillId(s.skill_id),
            name: s.name,
            body_snippet: truncate(&s.body, 200),
            score,
        }));
    }
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit as usize);
    hits.into_iter().map(|(_, h)| h).collect()
}

fn build_snippet(prompt: &Option<String>, text: &Option<String>) -> String {
    let mut out = String::new();
    if let Some(p) = prompt { out.push_str(&truncate(p, 120)); }
    if let Some(t) = text { if !out.is_empty() { out.push_str(" · "); } out.push_str(&truncate(t, 200)); }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() }
    else { let mut out: String = s.chars().take(n).collect(); out.push('…'); out }
}

pub struct SearchOutcome {
    pub hits: Vec<Hit>,
    pub degraded: bool,
    pub took_ms: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[test]
    fn recency_factor_recent_is_near_1() {
        let r = recency_factor(OffsetDateTime::now_utc());
        assert!(r > 0.999, "expected ~1.0, got {r}");
    }

    #[test]
    fn recency_factor_90_days_old_is_near_exp_neg_1() {
        let r = recency_factor(OffsetDateTime::now_utc() - time::Duration::days(90));
        assert!((r - (-1.0f32).exp()).abs() < 0.01, "expected ~0.368, got {r}");
    }

    #[test]
    fn final_score_blends_with_recency_and_project_boost() {
        let weights = BlendWeights::default();
        let ts = OffsetDateTime::now_utc();
        let s1 = final_score(1.0, 1.0, ts, weights, true);
        let s2 = final_score(1.0, 1.0, ts, weights, false);
        assert!((s1 - s2 - 0.3).abs() < 0.001);
    }

    #[test]
    fn rank_and_hydrate_orders_by_blended_score() {
        let now = OffsetDateTime::now_utc();
        let rank_a = RankedTurn {
            turn_id: uuid::Uuid::new_v4(), session_id: uuid::Uuid::new_v4(),
            ordinal: 0, ts: now, project_id: None,
            fts_score: 0.9, trgm_score: 0.0,
            user_prompt: Some("A".into()), assistant_text: None,
        };
        let rank_b = RankedTurn {
            turn_id: uuid::Uuid::new_v4(), session_id: uuid::Uuid::new_v4(),
            ordinal: 0, ts: now, project_id: None,
            fts_score: 0.5, trgm_score: 0.0,
            user_prompt: Some("B".into()), assistant_text: None,
        };
        let hits = rank_and_hydrate(vec![rank_a.clone(), rank_b.clone()], vec![], vec![], BlendWeights::default(), None, 10);
        assert_eq!(hits.len(), 2);
        match &hits[0] {
            Hit::Turn { turn_id, .. } => assert_eq!(turn_id.0, rank_a.turn_id),
            other => panic!("expected Turn, got {other:?}"),
        }
    }
}
