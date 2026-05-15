//! Deterministic synthetic corpus + qrels generator.
//!
//! Same `(seed, scale)` always yields the same files. The corpus is
//! sub-scaled by default (500 sessions) to keep the committed JSONL
//! under ~2 MB; pass `--scale=2000` for full spec parity at run-time.

use crate::corpus::{Corpus, FileDiffRow, SessionRow, ToolCallRow, TurnRow};
use crate::queries_bank::QUERIES;
use crate::types::{Judgment, QrelsFile, Query, QueryClass, QueriesFile};
use rand::distributions::WeightedIndex;
use rand::prelude::*;
use rand_chacha::ChaCha20Rng;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use teramind_core::types::file_diff::Attribution;
use time::OffsetDateTime;
use uuid::Uuid;

const SEED: u64 = 0xC0FF_EEC0_FFEE;

#[derive(Debug, Clone, Copy)]
struct Template {
    tag: &'static str,
    cwd: &'static str,
    seed_tokens: &'static [&'static str],
}

const TEMPLATES: &[Template] = &[
    Template { tag: "rust-web",    cwd: "/proj/rust-web",    seed_tokens: &["axum router", "tower middleware", "serde_json", "tokio spawn"] },
    Template { tag: "python-data", cwd: "/proj/python-data", seed_tokens: &["pandas DataFrame", "scikit-learn", "numpy vectorize", "matplotlib"] },
    Template { tag: "ts-react",    cwd: "/proj/ts-react",    seed_tokens: &["useState hook", "react-query", "tsx Component", "tailwind"] },
    Template { tag: "go-cli",      cwd: "/proj/go-cli",      seed_tokens: &["cobra command", "context.Context", "errgroup.Wait", "flag.StringVar"] },
];

pub fn generate_to(dest: &Path, scale: u32) -> anyhow::Result<()> {
    let mut rng = ChaCha20Rng::seed_from_u64(SEED);
    let (corpus, qrels) = build(&mut rng, scale);
    write_outputs(dest, &corpus, &qrels)?;
    println!(
        "teramind-search-eval: wrote {} sessions / {} turns / {} diffs to {}",
        corpus.sessions.len(),
        corpus.turns.len(),
        corpus.file_diffs.len(),
        dest.display(),
    );
    Ok(())
}

fn build(rng: &mut ChaCha20Rng, scale: u32) -> (Corpus, QrelsFile) {
    let triggers_by_query: Vec<(String, QueryClass, &'static [&'static str])> = QUERIES.iter()
        .map(|q| (q.id.to_string(), q.class, q.triggers))
        .collect();

    let template_weights: Vec<u32> = TEMPLATES.iter().map(|_| 1u32).collect();
    let template_dist = WeightedIndex::new(&template_weights).unwrap();

    let mut corpus = Corpus::default();
    let mut qrels: BTreeMap<String, Vec<Judgment>> = BTreeMap::new();
    let base_ts = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();

    for s_idx in 0..scale {
        let tpl = &TEMPLATES[template_dist.sample(rng)];
        let session_id = deterministic_uuid(rng);
        corpus.sessions.push(SessionRow {
            id: session_id,
            agent_kind: "claude_code".to_string(),
            cwd: tpl.cwd.to_string(),
            project_tag: tpl.tag.to_string(),
            started_at: base_ts + time::Duration::seconds(s_idx as i64 * 60),
        });

        let n_turns: u32 = rng.gen_range(2..=5);
        for t_idx in 0..n_turns {
            let turn_id = deterministic_uuid(rng);
            let chosen_query_idx: Option<usize> = if triggers_by_query.is_empty() {
                None
            } else if rng.gen_bool(0.20) {
                Some(rng.gen_range(0..triggers_by_query.len()))
            } else {
                None
            };

            let mut prompt = format!("{} -- task on {}", pick_one(rng, tpl.seed_tokens), tpl.tag);
            let mut assistant = format!("worked on the {} task", tpl.tag);

            if let Some(qi) = chosen_query_idx {
                let (qid, class, triggers) = &triggers_by_query[qi];
                let trig = pick_one(rng, triggers);
                match class {
                    QueryClass::NaturalLanguage => { prompt.push_str(&format!(" -- {}", trig)); }
                    QueryClass::StackTrace      => { assistant.push_str(&format!("\n{}", trig)); }
                    QueryClass::SymbolicPath    => { assistant.push_str(&format!(" using {}", trig)); }
                    QueryClass::CodeSnippet     => { /* planted in the file_diff below */ }
                    QueryClass::ToolTyped       => { /* planted in the tool_call below */ }
                }
                qrels.entry(qid.clone()).or_default().push(Judgment {
                    item: format!("turn:{}", turn_id),
                    grade: 2,
                });
            }

            corpus.turns.push(TurnRow {
                id: turn_id,
                session_id,
                ordinal: t_idx as i32,
                started_at: base_ts + time::Duration::seconds(s_idx as i64 * 60 + t_idx as i64),
                user_prompt: Some(prompt),
                assistant_text: Some(assistant),
                thinking: None,
            });

            let n_tools: u32 = rng.gen_range(0..=2);
            for tc_idx in 0..n_tools {
                let tool_id = deterministic_uuid(rng);
                let mut tool_output = format!("ran {} successfully", pick_one(rng, &["test", "bench", "build"]));
                let mut tool_name = pick_one(rng, &["Bash", "Read", "Grep"]);

                if let Some(qi) = chosen_query_idx {
                    let (qid, class, triggers) = &triggers_by_query[qi];
                    if matches!(class, QueryClass::ToolTyped) {
                        let trig = pick_one(rng, triggers);
                        tool_output = trig.to_string();
                        tool_name = "Edit";
                        qrels.entry(qid.clone()).or_default().push(Judgment {
                            item: format!("tool:{}", tool_id),
                            grade: 2,
                        });
                    }
                }

                corpus.tool_calls.push(ToolCallRow {
                    id: tool_id,
                    turn_id,
                    ordinal: tc_idx as i32,
                    name: tool_name.to_string(),
                    input: serde_json::json!({"x": tc_idx}),
                    output: tool_output,
                    started_at: base_ts + time::Duration::seconds(s_idx as i64 * 60 + t_idx as i64),
                });
            }

            if rng.gen_bool(0.4) {
                let diff_id = deterministic_uuid(rng);
                let rel_path = format!("src/{}.rs", pick_one(rng, &["lib", "util", "parser"]));
                let mut pre_excerpt = format!("fn old_{} {{}}", s_idx);
                let mut post_excerpt = format!("fn new_{} {{}}", s_idx);

                if let Some(qi) = chosen_query_idx {
                    let (qid, class, triggers) = &triggers_by_query[qi];
                    if matches!(class, QueryClass::CodeSnippet) {
                        let trig = pick_one(rng, triggers);
                        pre_excerpt = format!("{}\n{}", trig, pre_excerpt);
                        post_excerpt = format!("{}\n{}", trig, post_excerpt);
                        qrels.entry(qid.clone()).or_default().push(Judgment {
                            item: format!("diff:{}", diff_id),
                            grade: 2,
                        });
                    }
                }

                corpus.file_diffs.push(FileDiffRow {
                    id: diff_id,
                    session_id,
                    turn_id: Some(turn_id),
                    file_path: format!("{}/{}", tpl.cwd, rel_path),
                    rel_path,
                    attribution: Attribution::Agent,
                    language: Some("rust".into()),
                    pre_excerpt,
                    post_excerpt,
                    unified_diff: "@@ stub @@".into(),
                    captured_at: base_ts + time::Duration::seconds(s_idx as i64 * 60 + t_idx as i64),
                });
            }
        }
    }

    (corpus, QrelsFile { judgments: qrels })
}

fn deterministic_uuid(rng: &mut ChaCha20Rng) -> Uuid {
    let mut bytes = [0u8; 16];
    rng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    Uuid::from_bytes(bytes)
}

fn pick_one<'a>(rng: &mut ChaCha20Rng, slice: &'a [&'a str]) -> &'a str {
    slice[rng.gen_range(0..slice.len())]
}

fn write_outputs(dest: &Path, corpus: &Corpus, qrels: &QrelsFile) -> anyhow::Result<()> {
    let corpus_dir = dest.join("corpus");
    std::fs::create_dir_all(&corpus_dir)?;
    write_jsonl(&corpus_dir.join("sessions.jsonl"),   &corpus.sessions)?;
    write_jsonl(&corpus_dir.join("turns.jsonl"),      &corpus.turns)?;
    write_jsonl(&corpus_dir.join("tool_calls.jsonl"), &corpus.tool_calls)?;
    write_jsonl(&corpus_dir.join("file_diffs.jsonl"), &corpus.file_diffs)?;
    std::fs::write(dest.join("qrels.toml"), toml::to_string_pretty(qrels)?)?;
    std::fs::write(dest.join("queries.toml"), build_queries_toml()?)?;
    Ok(())
}

fn write_jsonl<T: Serialize>(path: &Path, rows: &[T]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    for r in rows {
        serde_json::to_writer(&mut f, r)?;
        f.write_all(b"\n")?;
    }
    f.flush()?;
    Ok(())
}

fn build_queries_toml() -> anyhow::Result<String> {
    let queries: Vec<Query> = QUERIES.iter().map(|q| Query {
        id: q.id.into(),
        class: q.class,
        text: q.text.into(),
    }).collect();
    Ok(toml::to_string_pretty(&QueriesFile { queries })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_is_deterministic_for_same_seed() {
        let mut a = ChaCha20Rng::seed_from_u64(SEED);
        let mut b = ChaCha20Rng::seed_from_u64(SEED);
        let (ca, qa) = build(&mut a, 50);
        let (cb, qb) = build(&mut b, 50);
        assert_eq!(ca.sessions.len(), cb.sessions.len());
        assert_eq!(
            ca.sessions.iter().map(|s| s.id).collect::<Vec<_>>(),
            cb.sessions.iter().map(|s| s.id).collect::<Vec<_>>(),
        );
        assert_eq!(qa.judgments.len(), qb.judgments.len());
    }

    #[test]
    fn generator_produces_expected_scale() {
        let mut rng = ChaCha20Rng::seed_from_u64(SEED);
        let (c, _) = build(&mut rng, 100);
        assert_eq!(c.sessions.len(), 100);
        assert!(!c.turns.is_empty());
    }
}
