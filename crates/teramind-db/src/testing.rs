//! Shared test fixtures. Each test binary gets ONE embedded Postgres
//! instance (started lazily); each test gets its own database within that
//! instance.

use crate::migrate;
use crate::pg_supervisor::PgSupervisor;
use crate::pool::DbPool;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::OnceCell;

static PG: OnceCell<SharedPg> = OnceCell::const_new();

struct SharedPg {
    sup: PgSupervisor,
    db_counter: AtomicU64,
    _data_dir: tempfile::TempDir,
}

async fn shared() -> &'static SharedPg {
    PG.get_or_init(|| async {
        let data_dir = tempfile::tempdir().expect("tempdir for shared PG");
        let sup = PgSupervisor::start(data_dir.path().to_path_buf(), "postgres")
            .await
            .expect("start shared embedded PG");
        SharedPg {
            sup,
            db_counter: AtomicU64::new(0),
            _data_dir: data_dir,
        }
    })
    .await
}

/// Freshly-migrated DbPool in an isolated database within the shared PG.
pub async fn fresh_pool() -> anyhow::Result<DbPool> {
    let pg = shared().await;
    let n = pg.db_counter.fetch_add(1, Ordering::SeqCst);
    let db_name = format!("test_db_{n}");

    let admin_opts = pg.sup.connect_options().database("postgres");
    let admin_pool = DbPool::connect(admin_opts).await?;
    sqlx::query(&format!("CREATE DATABASE {db_name}"))
        .execute(admin_pool.pg())
        .await?;
    drop(admin_pool);

    let opts = pg.sup.connect_options().database(&db_name);
    let pool = DbPool::connect(opts).await?;
    migrate::run(&pool).await?;
    Ok(pool)
}
