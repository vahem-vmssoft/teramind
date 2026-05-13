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
