use tempfile::TempDir;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

pub struct Fixture {
    pub sup: Option<PgSupervisor>,
    pub pool: DbPool,
    _tmp: TempDir,
}

impl Fixture {
    pub async fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let sup = PgSupervisor::start(tmp.path().to_path_buf(), "teramind_test")
            .await
            .unwrap();
        let pool = DbPool::connect(sup.connect_options()).await.unwrap();
        migrate::run(&pool).await.unwrap();
        Self {
            sup: Some(sup),
            pool,
            _tmp: tmp,
        }
    }
    pub async fn shutdown(mut self) {
        if let Some(s) = self.sup.take() {
            let _ = s.shutdown().await;
        }
    }
}

#[tokio::test]
async fn fixture_initializes() {
    let f = Fixture::new().await;
    let one: (i32,) = sqlx::query_as("SELECT 1")
        .fetch_one(f.pool.pg())
        .await
        .unwrap();
    assert_eq!(one.0, 1);
    f.shutdown().await;
}

#[tokio::test]
async fn agent_repo_upserts_and_finds() {
    let f = Fixture::new().await;
    let repo = teramind_db::repos::AgentRepo::new(f.pool.clone());
    let a1 = repo.upsert("claude_code", Some("0.1.0")).await.unwrap();
    let a2 = repo.upsert("claude_code", Some("0.1.0")).await.unwrap();
    assert_eq!(a1.id, a2.id);
    let found = repo
        .find("claude_code", Some("0.1.0"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, a1.id);
    f.shutdown().await;
}

#[tokio::test]
async fn project_repo_upserts_by_root_path() {
    let f = Fixture::new().await;
    let repo = teramind_db::repos::ProjectRepo::new(f.pool.clone());
    let p1 = repo
        .upsert_by_root("/home/dev/x", Some("git@github.com:org/x.git"), None)
        .await
        .unwrap();
    let p2 = repo
        .upsert_by_root("/home/dev/x", None, Some("X"))
        .await
        .unwrap();
    assert_eq!(p1.id, p2.id);
    assert_eq!(p2.git_remote.as_deref(), Some("git@github.com:org/x.git"));
    assert_eq!(p2.display_name.as_deref(), Some("X"));
    f.shutdown().await;
}

#[tokio::test]
async fn session_repo_inserts_and_ends() {
    let f = Fixture::new().await;
    let agents = teramind_db::repos::AgentRepo::new(f.pool.clone());
    let agent = agents.upsert("claude_code", Some("0.1.0")).await.unwrap();
    let repo = teramind_db::repos::SessionRepo::new(f.pool.clone());

    let now = time::OffsetDateTime::now_utc();
    let id = repo
        .insert(teramind_db::repos::session::NewSession {
            agent_id: agent.id,
            agent_session_id: Some("abc"),
            cwd: "/work",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "h",
            user_login: "u",
            started_at: now,
        })
        .await
        .unwrap();

    repo.end(id, now + time::Duration::seconds(60), "stop_hook")
        .await
        .unwrap();

    let (ended_at, end_reason): (Option<time::OffsetDateTime>, Option<String>) =
        sqlx::query_as("SELECT ended_at, end_reason FROM sessions WHERE id=$1")
            .bind(id.0)
            .fetch_one(f.pool.pg())
            .await
            .unwrap();
    assert!(ended_at.is_some());
    assert_eq!(end_reason.as_deref(), Some("stop_hook"));

    f.shutdown().await;
}

#[tokio::test]
async fn trace_repo_full_turn_lifecycle() {
    let f = Fixture::new().await;
    let agents = teramind_db::repos::AgentRepo::new(f.pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = teramind_db::repos::SessionRepo::new(f.pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let session_id = sessions
        .insert(teramind_db::repos::session::NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/w",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "h",
            user_login: "u",
            started_at: now,
        })
        .await
        .unwrap();

    let traces = teramind_db::repos::TraceRepo::new(f.pool.clone());
    let turn = traces
        .upsert_turn(session_id, 0, now, Some("hi"))
        .await
        .unwrap();
    let tc = traces
        .insert_tool_call_start(turn, 0, "Edit", &serde_json::json!({"x":1}), now)
        .await
        .unwrap();
    traces
        .finalize_tool_call(tc, "ok", false, 12)
        .await
        .unwrap();
    traces
        .finalize_turn(
            turn,
            now + time::Duration::seconds(1),
            Some("done"),
            None,
            Some("claude-opus-4-7"),
            Some(10),
            Some(5),
        )
        .await
        .unwrap();

    let row: (Option<String>, Option<String>, Option<i32>) =
        sqlx::query_as("SELECT assistant_text, model, output_tokens FROM turns WHERE id=$1")
            .bind(turn.0)
            .fetch_one(f.pool.pg())
            .await
            .unwrap();
    assert_eq!(row.0.as_deref(), Some("done"));
    assert_eq!(row.1.as_deref(), Some("claude-opus-4-7"));
    assert_eq!(row.2, Some(5));

    f.shutdown().await;
}

#[tokio::test]
async fn diff_repo_inserts_a_file_diff() {
    let f = Fixture::new().await;
    let agents = teramind_db::repos::AgentRepo::new(f.pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = teramind_db::repos::SessionRepo::new(f.pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let session_id = sessions
        .insert(teramind_db::repos::session::NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/w",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "h",
            user_login: "u",
            started_at: now,
        })
        .await
        .unwrap();
    let diffs = teramind_db::repos::DiffRepo::new(f.pool.clone());
    let id = diffs
        .insert(teramind_db::repos::diff::NewFileDiff {
            turn_id: None,
            session_id,
            file_path: "/w/x.rs",
            rel_path: "x.rs",
            attribution: teramind_core::types::file_diff::Attribution::Agent,
            language: Some("rust"),
            pre_excerpt: "a",
            post_excerpt: "b",
            unified_diff: "--- a\n+++ b\n",
            pre_hash: [1u8; 32],
            post_hash: [2u8; 32],
            byte_size: 1,
            captured_at: now,
        })
        .await
        .unwrap();
    let row: (i32,) = sqlx::query_as("SELECT byte_size FROM file_diffs WHERE id=$1")
        .bind(id.0)
        .fetch_one(f.pool.pg())
        .await
        .unwrap();
    assert_eq!(row.0, 1);
    f.shutdown().await;
}

#[tokio::test]
async fn skill_repo_upserts_authored() {
    let f = Fixture::new().await;
    let r = teramind_db::repos::SkillRepo::new(f.pool.clone());
    let id1 = r.upsert_authored("k", "d", "b1").await.unwrap();
    let id2 = r.upsert_authored("k", "d", "b2").await.unwrap();
    assert_eq!(id1, id2);
    let (body,): (String,) = sqlx::query_as("SELECT body FROM skills WHERE id=$1")
        .bind(id1.0)
        .fetch_one(f.pool.pg())
        .await
        .unwrap();
    assert_eq!(body, "b2");
    f.shutdown().await;
}

#[tokio::test]
async fn storage_stats_repo_inserts_and_counts() {
    let f = Fixture::new().await;
    let r = teramind_db::repos::StorageStatsRepo::new(f.pool.clone());
    r.insert(teramind_db::repos::storage_stats::Sample {
        pg_bytes: 100,
        jsonl_bytes: 200,
        session_count: 0,
        turn_count: 0,
        diff_count: 0,
    })
    .await
    .unwrap();
    assert_eq!(r.count_sessions().await.unwrap(), 0);
    f.shutdown().await;
}

#[tokio::test]
async fn trace_repo_accepts_caller_provided_tool_call_id() {
    let f = Fixture::new().await;
    let agents = teramind_db::repos::AgentRepo::new(f.pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = teramind_db::repos::SessionRepo::new(f.pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let session_id = sessions.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/w", project_id: None,
        parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u", started_at: now,
    }).await.unwrap();
    let trace = teramind_db::repos::TraceRepo::new(f.pool.clone());
    let turn = trace.upsert_turn(session_id, 0, now, None).await.unwrap();

    let chosen_id = teramind_core::ids::ToolCallId::new();
    let returned = trace.insert_tool_call_start_with_id(chosen_id, turn, 0, "Edit", &serde_json::json!({}), now).await.unwrap();
    assert_eq!(returned, chosen_id);

    let (db_id,): (uuid::Uuid,) = sqlx::query_as("SELECT id FROM tool_calls WHERE turn_id=$1 AND ordinal=0")
        .bind(turn.0).fetch_one(f.pool.pg()).await.unwrap();
    assert_eq!(db_id, chosen_id.0);

    f.shutdown().await;
}
