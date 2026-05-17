use teramind_core::types::file_diff::Attribution;
use teramind_db::repos::diff::NewFileDiff;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, DiffRepo, SearchRepo, SessionRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use time::OffsetDateTime;

#[tokio::test]
async fn diff_excerpts_for_cwd_files_filters_by_rel_path() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let diffs = DiffRepo::new(pool.clone());
    let search = SearchRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions
        .insert(NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/proj",
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

    let now = OffsetDateTime::now_utc();
    diffs
        .insert(NewFileDiff {
            turn_id: None,
            session_id: sid,
            file_path: "/proj/src/foo.rs",
            rel_path: "src/foo.rs",
            attribution: Attribution::Agent,
            language: Some("rust"),
            pre_excerpt: "old foo",
            post_excerpt: "new foo",
            unified_diff: "--- a/src/foo.rs\n+++ b/src/foo.rs\n-old foo\n+new foo\n",
            pre_hash: [0u8; 32],
            post_hash: [1u8; 32],
            byte_size: 7,
            captured_at: now,
        })
        .await?;
    diffs
        .insert(NewFileDiff {
            turn_id: None,
            session_id: sid,
            file_path: "/proj/src/bar.rs",
            rel_path: "src/bar.rs",
            attribution: Attribution::Agent,
            language: Some("rust"),
            pre_excerpt: "old bar",
            post_excerpt: "new bar",
            unified_diff: "--- a/src/bar.rs\n+++ b/src/bar.rs\n-old bar\n+new bar\n",
            pre_hash: [2u8; 32],
            post_hash: [3u8; 32],
            byte_size: 7,
            captured_at: now,
        })
        .await?;

    let hits = search
        .diff_excerpts_for_cwd_files(&["src/foo.rs".to_string()], 10)
        .await?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rel_path, "src/foo.rs");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vector_search_turns_returns_nearest_by_cosine() -> anyhow::Result<()> {
    use teramind_core::ids::TurnId;
    use teramind_db::repos::session::NewSession;
    use teramind_db::repos::{
        AgentRepo, EmbeddingRepo, SearchRepo, SessionRepo, ToEmbedRow, TraceRepo,
    };
    use time::OffsetDateTime;

    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(dir.path().to_path_buf(), "teramind")
        .await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let embed = EmbeddingRepo::new(pool.clone());
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

    let t_near = trace
        .upsert_turn_with_id(
            TurnId(uuid::Uuid::new_v4()),
            sid,
            0,
            OffsetDateTime::now_utc(),
            Some("near"),
        )
        .await?;
    let t_far = trace
        .upsert_turn_with_id(
            TurnId(uuid::Uuid::new_v4()),
            sid,
            1,
            OffsetDateTime::now_utc(),
            Some("far"),
        )
        .await?;

    let mut near_v = vec![0.0f32; 768];
    near_v[0] = 1.0;
    let mut far_v = vec![0.0f32; 768];
    far_v[1] = 1.0;

    embed
        .bulk_insert(
            &[ToEmbedRow {
                kind: "turn".into(),
                item_id: t_near.0,
                text: "near".into(),
            }],
            "test-model",
            768,
            &[near_v.clone()],
        )
        .await?;
    embed
        .bulk_insert(
            &[ToEmbedRow {
                kind: "turn".into(),
                item_id: t_far.0,
                text: "far".into(),
            }],
            "test-model",
            768,
            &[far_v.clone()],
        )
        .await?;

    let hits = search
        .vector_search_turns(&near_v, "test-model", 10)
        .await?;
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].turn_id, t_near.0);
    assert!(hits[0].semantic_score > hits[1].semantic_score);
    assert!(
        (hits[0].semantic_score - 1.0).abs() < 1e-6,
        "got {}",
        hits[0].semantic_score
    );

    sup.shutdown().await?;
    Ok(())
}
