use std::sync::Arc;
use std::time::Instant;
use teramind_core::ids::{FileDiffId, SessionId, SkillId, TurnId};
use teramind_core::types::Hit;
use teramind_db::repos::search::{RankedDiff, RankedSkill, RankedTurn, RankedWiki};
use time::OffsetDateTime;
use uuid::Uuid;

/// Configuration for the ranking blend. Spec §6.1 default weights.
#[derive(Debug, Clone, Copy)]
pub struct BlendWeights {
    pub fts: f32,
    pub trgm: f32,
    pub semantic: f32,
    pub recency: f32,
    pub project: f32,
}

impl Default for BlendWeights {
    fn default() -> Self {
        Self {
            fts: 0.6,
            trgm: 0.4,
            semantic: 0.0,
            recency: 0.2,
            project: 0.3,
        }
    }
}

pub fn final_score(
    fts: f32,
    trgm: f32,
    semantic: f32,
    ts: OffsetDateTime,
    weights: BlendWeights,
    same_project: bool,
) -> f32 {
    let recency_decay = recency_factor(ts);
    let project_boost = if same_project { 1.0 } else { 0.0 };
    weights.fts * fts
        + weights.trgm * trgm
        + weights.semantic * semantic
        + weights.recency * recency_decay
        + weights.project * project_boost
}

fn recency_factor(ts: OffsetDateTime) -> f32 {
    let age = OffsetDateTime::now_utc() - ts;
    let days = age.whole_seconds() as f32 / 86_400.0;
    (-days / 90.0).exp()
}

#[allow(clippy::too_many_arguments)]
pub fn rank_and_hydrate(
    fts_turns: Vec<RankedTurn>,
    trgm_diffs: Vec<RankedDiff>,
    trgm_skills: Vec<RankedSkill>,
    sem_turns: Vec<RankedTurn>,
    sem_diffs: Vec<RankedDiff>,
    fts_wikis: Vec<RankedWiki>,
    weights: BlendWeights,
    same_project_id: Option<Uuid>,
    limit: u32,
) -> Vec<Hit> {
    use std::collections::HashMap;

    // Merge turns by turn_id; semantic_score from sem_turns overrides if present.
    let mut by_turn: HashMap<Uuid, RankedTurn> = HashMap::new();
    for t in fts_turns {
        by_turn.insert(t.turn_id, t);
    }
    for t in sem_turns {
        by_turn
            .entry(t.turn_id)
            .and_modify(|existing| existing.semantic_score = t.semantic_score)
            .or_insert(t);
    }

    // Same for diffs.
    let mut by_diff: HashMap<Uuid, RankedDiff> = HashMap::new();
    for d in trgm_diffs {
        by_diff.insert(d.diff_id, d);
    }
    for d in sem_diffs {
        by_diff
            .entry(d.diff_id)
            .and_modify(|existing| existing.semantic_score = d.semantic_score)
            .or_insert(d);
    }

    let mut hits: Vec<(f32, Hit)> = Vec::new();
    for t in by_turn.into_values() {
        let same_p = same_project_id
            .map(|p| Some(p) == t.project_id)
            .unwrap_or(false);
        let score = final_score(
            t.fts_score,
            t.trgm_score,
            t.semantic_score,
            t.ts,
            weights,
            same_p,
        );
        let snippet = build_snippet(&t.user_prompt, &t.assistant_text);
        hits.push((
            score,
            Hit::Turn {
                turn_id: TurnId(t.turn_id),
                session_id: SessionId(t.session_id),
                ordinal: t.ordinal,
                snippet,
                score,
                ts: t.ts,
            },
        ));
    }
    for d in by_diff.into_values() {
        let same_p = same_project_id
            .map(|p| Some(p) == d.project_id)
            .unwrap_or(false);
        let score = final_score(0.0, d.trgm_score, d.semantic_score, d.ts, weights, same_p);
        let snippet = if d.post_excerpt.is_empty() {
            d.pre_excerpt.clone()
        } else {
            d.post_excerpt.clone()
        };
        hits.push((
            score,
            Hit::FileDiff {
                diff_id: FileDiffId(d.diff_id),
                rel_path: d.rel_path,
                hunk_snippet: snippet,
                score,
                ts: d.ts,
            },
        ));
    }
    for s in trgm_skills {
        let score = final_score(
            0.0,
            s.trgm_score,
            0.0,
            OffsetDateTime::now_utc(),
            weights,
            false,
        );
        hits.push((
            score,
            Hit::Skill {
                skill_id: SkillId(s.skill_id),
                name: s.name,
                body_snippet: truncate(&s.body, 200),
                score,
            },
        ));
    }
    for w in fts_wikis {
        let score = weights.fts * w.fts_score + weights.recency * recency_factor(w.ts);
        hits.push((
            score,
            Hit::WikiPage {
                page_id: teramind_core::ids::WikiPageId(w.page_id),
                session_id: SessionId(w.session_id),
                title: w.title,
                snippet: w.snippet,
                score,
                ts: w.ts,
            },
        ));
    }
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit as usize);
    hits.into_iter().map(|(_, h)| h).collect()
}

fn build_snippet(prompt: &Option<String>, text: &Option<String>) -> String {
    let mut out = String::new();
    if let Some(p) = prompt {
        out.push_str(&truncate(p, 120));
    }
    if let Some(t) = text {
        if !out.is_empty() {
            out.push_str(" · ");
        }
        out.push_str(&truncate(t, 200));
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }
}

pub struct SearchOutcome {
    pub hits: Vec<Hit>,
    pub degraded: bool,
    pub took_ms: u32,
}

use teramind_core::embed::EmbeddingProvider;
use teramind_core::types::{AutoRecallRequest, RecallRequest, SearchRequest};
use teramind_db::repos::SearchRepo;

pub async fn do_search(
    repo: &SearchRepo,
    provider: Option<Arc<dyn EmbeddingProvider>>,
    model: &str,
    weights: BlendWeights,
    req: &SearchRequest,
) -> Result<SearchOutcome, teramind_db::DbError> {
    let started = Instant::now();

    // Embed query if semantic weight is active and provider is available.
    let query_emb: Option<Vec<f32>> = if weights.semantic > 0.0 {
        match &provider {
            Some(p) => p
                .embed(std::slice::from_ref(&req.query))
                .await
                .ok()
                .and_then(|mut v| v.pop()),
            None => None,
        }
    } else {
        None
    };

    let (fts_res, trgm_diffs, trgm_skills, sem_turns, sem_diffs, fts_wikis) = tokio::try_join!(
        repo.fts_turns(&req.query, req.limit),
        repo.trgm_diffs(&req.query, req.limit),
        repo.trgm_skills(&req.query, req.limit),
        async {
            if let Some(v) = query_emb.as_ref() {
                repo.vector_search_turns(v, model, req.limit).await
            } else {
                Ok(vec![])
            }
        },
        async {
            if let Some(v) = query_emb.as_ref() {
                repo.vector_search_diffs(v, model, req.limit).await
            } else {
                Ok(vec![])
            }
        },
        repo.fts_wiki_pages(&req.query, req.limit),
    )?;

    let degraded = weights.semantic > 0.0 && query_emb.is_none();
    let hits = rank_and_hydrate(
        fts_res,
        trgm_diffs,
        trgm_skills,
        sem_turns,
        sem_diffs,
        fts_wikis,
        weights,
        None,
        req.limit,
    );
    Ok(SearchOutcome {
        hits,
        degraded,
        took_ms: started.elapsed().as_millis() as u32,
    })
}

pub async fn do_recall(
    repo: &SearchRepo,
    req: &RecallRequest,
) -> Result<SearchOutcome, teramind_db::DbError> {
    let started = Instant::now();
    let symbol_query = req.symbols.join(" ");
    let stacktrace_query = req.stack_traces.join(" ");
    let path_query = req.file_paths.join(" ");

    let (fts_sym, fts_st, trgm_paths) = tokio::try_join!(
        async {
            if symbol_query.is_empty() {
                Ok::<_, teramind_db::DbError>(vec![])
            } else {
                repo.fts_turns(&symbol_query, req.limit).await
            }
        },
        async {
            if stacktrace_query.is_empty() {
                Ok::<_, teramind_db::DbError>(vec![])
            } else {
                repo.fts_turns(&stacktrace_query, req.limit).await
            }
        },
        async {
            if path_query.is_empty() {
                Ok::<_, teramind_db::DbError>(vec![])
            } else {
                repo.trgm_diffs(&path_query, req.limit).await
            }
        },
    )?;
    let merged: Vec<_> = fts_sym.into_iter().chain(fts_st).collect();
    let hits = rank_and_hydrate(
        merged,
        trgm_paths,
        vec![],
        vec![],
        vec![],
        vec![],
        BlendWeights::default(),
        None,
        req.limit,
    );
    Ok(SearchOutcome {
        hits,
        degraded: false,
        took_ms: started.elapsed().as_millis() as u32,
    })
}

use crate::services::grep_fallback;
use std::path::Path;

pub async fn do_search_with_fallback(
    repo: &SearchRepo,
    jsonl_dir: &Path,
    provider: Option<Arc<dyn EmbeddingProvider>>,
    model: &str,
    weights: BlendWeights,
    req: &SearchRequest,
) -> SearchOutcome {
    match do_search(repo, provider, model, weights, req).await {
        Ok(o) => o,
        Err(_) => {
            let started = Instant::now();
            let hits = grep_fallback::run(jsonl_dir, &req.query, req.limit)
                .await
                .unwrap_or_default();
            SearchOutcome {
                hits,
                degraded: true,
                took_ms: started.elapsed().as_millis() as u32,
            }
        }
    }
}

fn shorten_uuid(s: &str) -> String {
    s.chars().take(8).collect::<String>() + "..."
}

pub fn render_auto_recall_md(
    recent: &[teramind_db::repos::search::RankedTurn],
    diffs: &[teramind_db::repos::search::RankedDiff],
    latest_wiki: Option<&teramind_db::repos::WikiPage>,
) -> String {
    let mut out = String::new();
    if let Some(w) = latest_wiki {
        out.push_str("## Most recent session summary\n\n");
        out.push_str(&format!(
            "> *Generated {} from session {}*\n\n",
            w.generated_at.date(),
            shorten_uuid(&w.session_id.0.to_string()),
        ));
        let body = if w.content.len() > 1500 {
            let mut end = 1500;
            while end > 0 && !w.content.is_char_boundary(end) {
                end -= 1;
            }
            let mut t = w.content[..end].to_string();
            t.push_str("\n\n*(truncated; see `mcp__teramind__wiki` for full page)*");
            t
        } else {
            w.content.clone()
        };
        out.push_str(&body);
        out.push_str("\n\n");
    }
    if !recent.is_empty() {
        out.push_str("## Recent Teramind context\n\n");
        for t in recent {
            let prompt_snippet = t.user_prompt.as_deref().unwrap_or("(no prompt)");
            let text_snippet = t.assistant_text.as_deref().unwrap_or("");
            out.push_str(&format!(
                "- **{}**: {} · {}\n",
                t.ts.date(),
                truncate(prompt_snippet, 80),
                truncate(text_snippet, 120),
            ));
        }
        out.push('\n');
    }
    if !diffs.is_empty() {
        out.push_str("## Recent diffs in this project\n\n");
        for d in diffs {
            out.push_str(&format!(
                "- `{}` @ {}: {}\n",
                d.rel_path,
                d.ts.date(),
                truncate(&d.post_excerpt, 120),
            ));
        }
    }
    out
}

/// Returns the "Relevant codified skills" section of the SessionStart digest
/// (or empty string if no skills match the cwd).
pub async fn relevant_codified_skills(
    skills: &teramind_db::repos::SkillRepo,
    cwd: &str,
    top_k: usize,
) -> String {
    let rows = match skills.list_codified_for_cwd(cwd, 50).await {
        Ok(rs) => rs,
        Err(_) => return String::new(),
    };
    let matched: Vec<_> = rows.into_iter()
        .filter(|(_, _, _, applies, _)| crate::services::codify::glob::matches_any(applies, cwd))
        .take(top_k)
        .collect();
    if matched.is_empty() { return String::new(); }
    let mut out = String::from("\n## Relevant codified skills\n\n");
    for (_id, name, desc, _, seeded_from) in &matched {
        out.push_str(&format!(
            "- **{name}** — {desc} _(seeded from {seeded_from} sessions)_\n"
        ));
    }
    out.push_str("\nTo recall the full body of any skill: `mcp__teramind__search` with the skill name.\n");
    out
}

pub async fn do_auto_recall(
    repo: &SearchRepo,
    wiki_repo: &teramind_db::repos::WikiRepo,
    skills_repo: &teramind_db::repos::SkillRepo,
    req: &AutoRecallRequest,
) -> Result<String, teramind_db::DbError> {
    let (recent, diffs) = tokio::try_join!(
        repo.recent_turns_in_project(None, &req.cwd, req.limit),
        repo.diff_excerpts_for_cwd_files(&req.cwd_files, req.limit),
    )?;
    let latest_wiki = wiki_repo.latest_for_cwd(&req.cwd).await.ok().flatten();
    if recent.is_empty() && diffs.is_empty() && latest_wiki.is_none() {
        return Ok(String::new());
    }
    let mut out = relevant_codified_skills(skills_repo, &req.cwd, 5).await;
    out.push_str(&render_auto_recall_md(&recent, &diffs, latest_wiki.as_ref()));
    Ok(out)
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
        assert!(
            (r - (-1.0f32).exp()).abs() < 0.01,
            "expected ~0.368, got {r}"
        );
    }

    #[test]
    fn final_score_blends_with_recency_and_project_boost() {
        let weights = BlendWeights::default();
        let ts = OffsetDateTime::now_utc();
        let s1 = final_score(1.0, 1.0, 0.0, ts, weights, true);
        let s2 = final_score(1.0, 1.0, 0.0, ts, weights, false);
        assert!((s1 - s2 - 0.3).abs() < 0.001);
    }

    #[test]
    fn semantic_weight_contributes_to_score() {
        let weights = BlendWeights {
            fts: 0.0,
            trgm: 0.0,
            semantic: 1.0,
            recency: 0.0,
            project: 0.0,
        };
        let ts = OffsetDateTime::now_utc();
        let s = final_score(0.0, 0.0, 0.5, ts, weights, false);
        assert!((s - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rank_and_hydrate_orders_by_blended_score() {
        let now = OffsetDateTime::now_utc();
        let rank_a = RankedTurn {
            turn_id: uuid::Uuid::new_v4(),
            session_id: uuid::Uuid::new_v4(),
            ordinal: 0,
            ts: now,
            project_id: None,
            fts_score: 0.9,
            trgm_score: 0.0,
            semantic_score: 0.0,
            user_prompt: Some("A".into()),
            assistant_text: None,
        };
        let rank_b = RankedTurn {
            turn_id: uuid::Uuid::new_v4(),
            session_id: uuid::Uuid::new_v4(),
            ordinal: 0,
            ts: now,
            project_id: None,
            fts_score: 0.5,
            trgm_score: 0.0,
            semantic_score: 0.0,
            user_prompt: Some("B".into()),
            assistant_text: None,
        };
        let hits = rank_and_hydrate(
            vec![rank_a.clone(), rank_b.clone()],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            BlendWeights::default(),
            None,
            10,
        );
        assert_eq!(hits.len(), 2);
        match &hits[0] {
            Hit::Turn { turn_id, .. } => assert_eq!(turn_id.0, rank_a.turn_id),
            other => panic!("expected Turn, got {other:?}"),
        }
    }

    #[test]
    fn render_auto_recall_md_includes_diffs_section_when_present() {
        let recent_turns: Vec<RankedTurn> = vec![RankedTurn {
            turn_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            ordinal: 0,
            ts: OffsetDateTime::now_utc(),
            project_id: None,
            fts_score: 0.0,
            trgm_score: 0.0,
            semantic_score: 0.0,
            user_prompt: Some("fix bug".into()),
            assistant_text: Some("done".into()),
        }];
        let diff_hits: Vec<teramind_db::repos::search::RankedDiff> =
            vec![teramind_db::repos::search::RankedDiff {
                diff_id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                rel_path: "src/foo.rs".into(),
                ts: OffsetDateTime::now_utc(),
                project_id: None,
                trgm_score: 0.0,
                semantic_score: 0.0,
                pre_excerpt: "old foo".into(),
                post_excerpt: "new foo".into(),
            }];
        let md = render_auto_recall_md(&recent_turns, &diff_hits, None);
        assert!(md.contains("Recent Teramind context"));
        assert!(md.contains("fix bug"));
        assert!(md.contains("Recent diffs"));
        assert!(md.contains("src/foo.rs"));
    }

    #[test]
    fn render_auto_recall_md_includes_wiki_section_when_present() {
        use teramind_core::ids::{SessionId, WikiPageId};
        use teramind_db::repos::WikiPage;
        let wiki = WikiPage {
            id: WikiPageId(uuid::Uuid::new_v4()),
            session_id: SessionId(uuid::Uuid::new_v4()),
            model: "ollama:qwen3.6:latest".into(),
            content: "# Summary\nThe agent refactored JWT...".into(),
            input_tokens: 100,
            output_tokens: 50,
            generated_at: OffsetDateTime::now_utc(),
        };
        let md = render_auto_recall_md(&[], &[], Some(&wiki));
        assert!(md.contains("Most recent session summary"));
        assert!(md.contains("refactored JWT"));
    }

    #[test]
    fn render_auto_recall_md_truncates_long_wiki() {
        use teramind_core::ids::{SessionId, WikiPageId};
        use teramind_db::repos::WikiPage;
        let wiki = WikiPage {
            id: WikiPageId(uuid::Uuid::new_v4()),
            session_id: SessionId(uuid::Uuid::new_v4()),
            model: "test".into(),
            content: "A".repeat(5000),
            input_tokens: 0,
            output_tokens: 0,
            generated_at: OffsetDateTime::now_utc(),
        };
        let md = render_auto_recall_md(&[], &[], Some(&wiki));
        assert!(md.contains("(truncated"));
    }

    #[test]
    fn rank_and_hydrate_emits_wiki_page_hits() {
        let now = OffsetDateTime::now_utc();
        let w = teramind_db::repos::search::RankedWiki {
            page_id: uuid::Uuid::new_v4(),
            session_id: uuid::Uuid::new_v4(),
            title: "Refactor".into(),
            snippet: "The agent refactored JWT".into(),
            fts_score: 0.7,
            ts: now,
        };
        let hits = rank_and_hydrate(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![w],
            BlendWeights::default(),
            None,
            10,
        );
        assert_eq!(hits.len(), 1);
        match &hits[0] {
            Hit::WikiPage { title, .. } => assert_eq!(title, "Refactor"),
            other => panic!("expected WikiPage hit, got {:?}", other),
        }
    }
}
