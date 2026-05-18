//! L3: writing a wiki_page joins traces_fts so search hits the summary.

use teramind_core::ids::TurnId;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SearchRepo, SessionRepo, TraceRepo, WikiRepo};
use time::OffsetDateTime;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_finds_wiki_via_traces_fts() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let wiki = WikiRepo::new(pool.clone());
    let search = SearchRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions
        .insert(NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/p",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "h",
            user_login: "u",
            started_at: OffsetDateTime::now_utc(),
            user_id: None,
            device_id: None,
        })
        .await?;
    let _tid = trace
        .upsert_turn_with_id(
            TurnId(uuid::Uuid::new_v4()),
            sid,
            0,
            OffsetDateTime::now_utc(),
            Some("unrelated"),
        )
        .await?;

    // Insert a wiki with a unique token. The 'thirteen-banana-tower' phrase
    // appears nowhere in turns/tool_calls/file_diffs.
    wiki.upsert(
        sid,
        "test-model",
        "# Summary\nThe agent applied the thirteen-banana-tower refactor.",
        50,
        50,
    )
    .await?;

    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts")
        .execute(pool.pg())
        .await?;

    // 1. FTS over turns now hits the synthetic phrase via the wiki UNION.
    let turn_hits = search.fts_turns("thirteen-banana-tower", 10).await?;
    assert!(
        !turn_hits.is_empty(),
        "wiki content should join traces_fts so turn-level FTS finds it"
    );

    // 2. Direct wiki search returns the page.
    let wiki_hits = search.fts_wiki_pages("thirteen-banana-tower", 10).await?;
    assert_eq!(wiki_hits.len(), 1);
    assert!(wiki_hits[0].title.contains("Summary"));

    Ok(())
}
