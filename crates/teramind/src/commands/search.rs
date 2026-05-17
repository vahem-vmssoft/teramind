use crate::ipc;
use teramind_core::types::{Hit, SearchRequest};
use teramind_ipc::proto::{Request, Response};

pub async fn run(query: String, limit: u32, json: bool, _grep: bool) -> anyhow::Result<()> {
    // _grep is reserved for future "force grep" wiring. v1 always lets the daemon decide.
    let resp = ipc::request(Request::Search(SearchRequest { query, limit }), 10_000).await?;
    let results = match resp {
        Response::SearchResults(s) => s,
        Response::Error(e) => {
            eprintln!("error: {e}");
            return Ok(());
        }
        other => {
            eprintln!("unexpected: {other:?}");
            return Ok(());
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }
    if results.degraded {
        eprintln!("(degraded: Postgres unreachable, served from JSONL via grep)");
    }
    eprintln!("({} hits in {} ms)", results.hits.len(), results.took_ms);
    for (i, h) in results.hits.iter().enumerate() {
        match h {
            Hit::Turn { session_id, ordinal, snippet, score, ts, .. } =>
                println!("{i:3}. [turn]    {ts}  session={session_id}#{ordinal}  score={score:.3}\n      {snippet}"),
            Hit::ToolCall { name, input_snippet, output_snippet, score, ts, .. } =>
                println!("{i:3}. [tool {name}]  {ts}  score={score:.3}\n      in:  {input_snippet}\n      out: {output_snippet}"),
            Hit::FileDiff { rel_path, hunk_snippet, score, ts, .. } =>
                println!("{i:3}. [diff]    {ts}  {rel_path}  score={score:.3}\n      {hunk_snippet}"),
            Hit::Skill { name, body_snippet, score, .. } =>
                println!("{i:3}. [skill]   {name}  score={score:.3}\n      {body_snippet}"),
            Hit::WikiPage { title, snippet, score, ts, .. } =>
                println!("{i:3}. [wiki]    {ts}  {title}  score={score:.3}\n      {snippet}"),
        }
    }
    Ok(())
}
