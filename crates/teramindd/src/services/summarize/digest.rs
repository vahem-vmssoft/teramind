//! Pure digest builder. Takes a SessionSnapshot, returns a Markdown
//! string capped at `char_budget`. No I/O, no async.

pub use teramind_core::ids::{SessionId, TurnId};
pub use teramind_core::summarize::{FileDiffRow, SessionSnapshot, ToolCallRow, TurnRow};
pub use teramind_core::types::file_diff::Attribution;
pub use time::OffsetDateTime;

/// Build a Markdown digest from the snapshot. Output length <= `char_budget`.
/// Sections are dropped in priority order when over budget.
pub fn build(snapshot: &SessionSnapshot, char_budget: usize) -> String {
    let mut sections = Vec::new();
    sections.push(("header".to_string(), render_header(snapshot)));
    let tools = render_tool_usage(snapshot);
    if !tools.is_empty() {
        sections.push(("tools".to_string(), tools));
    }
    let files = render_files_changed(snapshot);
    if !files.is_empty() {
        sections.push(("files".to_string(), files));
    }
    let prompts = render_key_prompts(snapshot);
    if !prompts.is_empty() {
        sections.push(("prompts".to_string(), prompts));
    }
    let outputs = render_key_outputs(snapshot);
    if !outputs.is_empty() {
        sections.push(("outputs".to_string(), outputs));
    }
    let errors = render_tool_errors(snapshot);
    if !errors.is_empty() {
        sections.push(("errors".to_string(), errors));
    }
    let diffs = render_diff_samples(snapshot);
    if !diffs.is_empty() {
        sections.push(("diffs".to_string(), diffs));
    }

    enforce_budget(sections, char_budget)
}

// Priority drop order when over budget:
//   1. "diffs"      (highest cost / lowest priority)
//   2. "outputs"
//   3. "prompts"
//   4. "errors"
// Header / tools / files are always kept; the file list itself is truncated
// to 10 entries as a final fallback.
const DROP_ORDER: &[&str] = &["diffs", "outputs", "prompts", "errors"];

fn enforce_budget(mut sections: Vec<(String, String)>, budget: usize) -> String {
    let mut result = join(&sections);
    let mut idx = 0;
    while result.len() > budget && idx < DROP_ORDER.len() {
        let target = DROP_ORDER[idx];
        sections.retain(|(name, _)| name != target);
        result = join(&sections);
        idx += 1;
    }
    if result.len() > budget {
        // Final fallback: truncate the "files" section to the first 10 bullets.
        if let Some(files_idx) = sections.iter().position(|(n, _)| n == "files") {
            sections[files_idx].1 = truncate_bullets(&sections[files_idx].1, 10);
            result = join(&sections);
        }
    }
    if result.len() > budget {
        result = truncate_to_char_boundary(&result, budget);
    }
    result
}

fn join(sections: &[(String, String)]) -> String {
    sections
        .iter()
        .map(|(_, body)| body.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn truncate_to_char_boundary(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn truncate_bullets(s: &str, max_bullets: usize) -> String {
    let mut out = String::new();
    let mut bullet_count = 0;
    for line in s.lines() {
        if line.trim_start().starts_with("- ") {
            if bullet_count >= max_bullets {
                continue;
            }
            bullet_count += 1;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn render_header(s: &SessionSnapshot) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    writeln!(out, "# Session digest\n").unwrap();
    writeln!(out, "- session_id: {}", s.session_id.0).unwrap();
    writeln!(out, "- cwd: {}", s.cwd).unwrap();
    let dur = s.duration_secs();
    writeln!(out, "- duration: {}m {}s", dur / 60, dur % 60).unwrap();
    if let (Some(b), Some(h)) = (&s.git_branch, &s.git_head) {
        writeln!(
            out,
            "- git branch / head: {} at {}",
            b,
            &h[..h.len().min(7)]
        )
        .unwrap();
    }
    writeln!(out, "- ended: {}", s.end_reason).unwrap();
    writeln!(
        out,
        "- turns: {}    tool calls: {}    files changed: {}",
        s.turns.len(),
        s.tool_calls.len(),
        s.file_diffs.len(),
    )
    .unwrap();
    out
}

fn render_tool_usage(s: &SessionSnapshot) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    if s.tool_calls.is_empty() {
        return String::new();
    }
    let mut counts: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    for tc in &s.tool_calls {
        let e = counts.entry(tc.name.as_str()).or_insert((0, 0));
        e.0 += 1;
        if tc.is_error {
            e.1 += 1;
        }
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by_key(|b| std::cmp::Reverse(b.1 .0));
    ranked.truncate(5);
    let mut out = String::new();
    writeln!(out, "## Tool usage (top 5 by count)\n").unwrap();
    for (name, (n, errs)) in ranked {
        if errs > 0 {
            writeln!(out, "- {} x{}  (errors: {})", name, n, errs).unwrap();
        } else {
            writeln!(out, "- {} x{}", name, n).unwrap();
        }
    }
    out
}

fn render_files_changed(s: &SessionSnapshot) -> String {
    use std::fmt::Write;
    if s.file_diffs.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    writeln!(out, "## Files changed\n").unwrap();
    for d in &s.file_diffs {
        let plus = d
            .unified_diff
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .count();
        let minus = d
            .unified_diff
            .lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .count();
        let attr = match d.attribution {
            Attribution::Agent => "agent",
            Attribution::Human => "human",
        };
        let lang = d.language.as_deref().unwrap_or("text");
        writeln!(
            out,
            "- {} ({}, {}) — (+{}, -{})",
            d.rel_path, lang, attr, plus, minus
        )
        .unwrap();
    }
    out
}

fn render_key_prompts(s: &SessionSnapshot) -> String {
    use std::fmt::Write;
    let mut prompts: Vec<&str> = s
        .turns
        .iter()
        .filter_map(|t| t.user_prompt.as_deref())
        .filter(|p| !p.trim().is_empty())
        .collect();
    if prompts.is_empty() {
        return String::new();
    }
    prompts.sort_by_key(|p| std::cmp::Reverse(p.len()));
    prompts.truncate(5);
    let mut out = String::new();
    writeln!(out, "## Key prompts (longest, up to 5)\n").unwrap();
    for (i, p) in prompts.iter().enumerate() {
        writeln!(out, "{}. > {:?}", i + 1, truncate_for_quote(p, 400)).unwrap();
    }
    out
}

fn render_key_outputs(s: &SessionSnapshot) -> String {
    use std::fmt::Write;
    let mut outs: Vec<&str> = s
        .turns
        .iter()
        .filter_map(|t| t.assistant_text.as_deref())
        .filter(|p| !p.trim().is_empty())
        .collect();
    if outs.is_empty() {
        return String::new();
    }
    outs.sort_by_key(|p| std::cmp::Reverse(p.len()));
    outs.truncate(5);
    let mut out = String::new();
    writeln!(out, "## Key assistant outputs (longest, up to 5)\n").unwrap();
    for (i, p) in outs.iter().enumerate() {
        writeln!(out, "{}. > {:?}", i + 1, truncate_for_quote(p, 400)).unwrap();
    }
    out
}

fn render_tool_errors(s: &SessionSnapshot) -> String {
    use std::fmt::Write;
    let errs: Vec<_> = s
        .tool_calls
        .iter()
        .filter(|tc| tc.is_error)
        .take(3)
        .collect();
    if errs.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    writeln!(out, "## Notable tool errors (up to 3)\n").unwrap();
    for tc in errs {
        let snippet = truncate_for_quote(&tc.output, 200);
        writeln!(out, "- {}: {:?}", tc.name, snippet).unwrap();
    }
    out
}

fn render_diff_samples(s: &SessionSnapshot) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    if s.file_diffs.is_empty() {
        return String::new();
    }
    let mut by_path: BTreeMap<&str, (usize, &FileDiffRow)> = BTreeMap::new();
    for d in &s.file_diffs {
        let churn = d.unified_diff.lines().count();
        by_path
            .entry(d.rel_path.as_str())
            .and_modify(|e| {
                if churn > e.0 {
                    *e = (churn, d);
                }
            })
            .or_insert((churn, d));
    }
    let mut ranked: Vec<_> = by_path.into_iter().collect();
    ranked.sort_by_key(|b| std::cmp::Reverse(b.1 .0));
    ranked.truncate(2);
    let mut out = String::new();
    writeln!(out, "## Diff samples (one per file, top 2 by churn)\n").unwrap();
    for (_, (_, d)) in ranked {
        let fence = match d.language.as_deref().unwrap_or("text") {
            l if !l.is_empty() => l,
            _ => "text",
        };
        writeln!(out, "```{}", fence).unwrap();
        writeln!(out, "// {}", d.rel_path).unwrap();
        let snippet = truncate_to_char_boundary(&d.unified_diff, 1200);
        out.push_str(&snippet);
        if !snippet.ends_with('\n') {
            out.push('\n');
        }
        writeln!(out, "```\n").unwrap();
    }
    out
}

fn truncate_for_quote(s: &str, max: usize) -> String {
    let single_line: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if single_line.chars().count() <= max {
        single_line
    } else {
        let mut out: String = single_line.chars().take(max).collect();
        out.push_str("...");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_with_turns(n: usize) -> SessionSnapshot {
        let mut turns = Vec::new();
        for i in 0..n {
            turns.push(TurnRow {
                id: TurnId(uuid::Uuid::new_v4()),
                ordinal: i as i32,
                user_prompt: Some(format!("prompt {i}")),
                assistant_text: Some(format!("response {i}")),
                thinking: None,
                started_at: OffsetDateTime::now_utc(),
            });
        }
        SessionSnapshot {
            session_id: SessionId(uuid::Uuid::new_v4()),
            cwd: "/proj".into(),
            started_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            ended_at: OffsetDateTime::from_unix_timestamp(1_700_002_000).unwrap(),
            end_reason: "stop_hook".into(),
            git_branch: Some("main".into()),
            git_head: Some("abc1234567890".into()),
            turns,
            tool_calls: Vec::new(),
            file_diffs: Vec::new(),
        }
    }

    #[test]
    fn build_respects_char_budget() {
        let s = snapshot_with_turns(50);
        let d = build(&s, 1024);
        assert!(d.len() <= 1024, "len={}", d.len());
        assert!(
            d.starts_with("# Session digest"),
            "got: {}",
            &d[..d.len().min(200)]
        );
    }

    #[test]
    fn build_is_deterministic() {
        let s = snapshot_with_turns(5);
        let a = build(&s, 8192);
        let b = build(&s, 8192);
        assert_eq!(a, b);
    }

    #[test]
    fn build_priority_drop_order() {
        let mut s = snapshot_with_turns(20);
        // Force diff samples to exist.
        s.file_diffs.push(FileDiffRow {
            turn_id: None,
            rel_path: "a.rs".into(),
            language: Some("rust".into()),
            attribution: Attribution::Agent,
            unified_diff: "--- a\n+++ b\n@@ x\n-old\n+new\n".into(),
            pre_excerpt: "old".into(),
            post_excerpt: "new".into(),
        });
        let unlimited = build(&s, 32768);
        let limited = build(&s, 600);
        assert!(unlimited.contains("Diff samples"));
        assert!(
            !limited.contains("Diff samples"),
            "diffs should be dropped first at small budget; got:\n{}",
            limited
        );
    }

    #[test]
    fn truncate_to_char_boundary_handles_multi_byte() {
        let s = "héllo"; // 'é' is 2 bytes
        let t = truncate_to_char_boundary(s, 2);
        assert!(s.starts_with(&t));
        assert!(t == "h" || t == "hé");
    }

    use proptest::prelude::*;
    proptest! {
        #[test]
        fn build_length_never_exceeds_budget(
            turn_count in 0usize..30usize,
            budget in 256usize..16384usize,
        ) {
            let s = snapshot_with_turns(turn_count);
            let d = build(&s, budget);
            prop_assert!(d.len() <= budget, "len={} > budget={}", d.len(), budget);
        }
    }
}
