use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use tempfile::TempDir;

pub struct Fixture {
    pub sup: Option<PgSupervisor>,
    pub pool: DbPool,
    _tmp: TempDir,
}

impl Fixture {
    pub async fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let sup = PgSupervisor::start(tmp.path().to_path_buf(), "teramind_test").await.unwrap();
        let pool = DbPool::connect(sup.connect_options()).await.unwrap();
        migrate::run(&pool).await.unwrap();
        Self { sup: Some(sup), pool, _tmp: tmp }
    }
    pub async fn shutdown(mut self) {
        if let Some(s) = self.sup.take() { let _ = s.shutdown().await; }
    }
}

#[tokio::test]
async fn fixture_initializes() {
    let f = Fixture::new().await;
    let one: (i32,) = sqlx::query_as("SELECT 1").fetch_one(f.pool.pg()).await.unwrap();
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
    let found = repo.find("claude_code", Some("0.1.0")).await.unwrap().unwrap();
    assert_eq!(found.id, a1.id);
    f.shutdown().await;
}

#[tokio::test]
async fn project_repo_upserts_by_root_path() {
    let f = Fixture::new().await;
    let repo = teramind_db::repos::ProjectRepo::new(f.pool.clone());
    let p1 = repo.upsert_by_root("/home/dev/x", Some("git@github.com:org/x.git"), None).await.unwrap();
    let p2 = repo.upsert_by_root("/home/dev/x", None, Some("X")).await.unwrap();
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
    let id = repo.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id,
        agent_session_id: Some("abc"),
        cwd: "/work",
        project_id: None,
        parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: now,
    }).await.unwrap();

    repo.end(id, now + time::Duration::seconds(60), "stop_hook").await.unwrap();

    let (ended_at, end_reason): (Option<time::OffsetDateTime>, Option<String>) = sqlx::query_as(
        "SELECT ended_at, end_reason FROM sessions WHERE id=$1")
        .bind(id.0)
        .fetch_one(f.pool.pg()).await.unwrap();
    assert!(ended_at.is_some());
    assert_eq!(end_reason.as_deref(), Some("stop_hook"));

    f.shutdown().await;
}
