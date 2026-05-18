//! Shared test fixtures. Each test binary gets ONE Postgres instance
//! (started lazily); each test gets its own database within that instance.
//!
//! Backend selection:
//! - If `TERAMIND_TEST_PG_URL` is set, use that external Postgres.
//! - Otherwise, spin up the embedded supervisor (the default; works in CI).
//!
//! The external path is dramatically faster: it skips `initdb`, PG-process
//! boot, and pgvector extraction. Recommended for local dev when a system
//! Postgres is already running with pgvector available.

use crate::migrate;
use crate::pg_supervisor::PgSupervisor;
use crate::pool::DbPool;
use sqlx::postgres::PgConnectOptions;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::OnceCell;

static PG: OnceCell<SharedPg> = OnceCell::const_new();

enum Backend {
    Embedded {
        sup: PgSupervisor,
        _data_dir: tempfile::TempDir,
    },
    External {
        admin_opts: PgConnectOptions,
    },
}

struct SharedPg {
    backend: Backend,
    db_counter: AtomicU64,
}

impl SharedPg {
    /// Connect options to the supervisor's "admin" database (one we can
    /// issue CREATE DATABASE against). For embedded PG that's the `postgres`
    /// database the supervisor created; for external it's whatever the URL's
    /// path component points at.
    fn admin_opts(&self) -> PgConnectOptions {
        match &self.backend {
            Backend::Embedded { sup, .. } => sup.connect_options().database("postgres"),
            Backend::External { admin_opts } => admin_opts.clone(),
        }
    }

    fn db_opts(&self, db: &str) -> PgConnectOptions {
        match &self.backend {
            Backend::Embedded { sup, .. } => sup.connect_options().database(db),
            Backend::External { admin_opts } => admin_opts.clone().database(db),
        }
    }
}

async fn shared() -> &'static SharedPg {
    PG.get_or_init(|| async {
        if let Ok(url) = std::env::var("TERAMIND_TEST_PG_URL") {
            init_external(&url).await
        } else {
            init_embedded().await
        }
    })
    .await
}

async fn init_embedded() -> SharedPg {
    let data_dir = tempfile::tempdir().expect("tempdir for shared PG");
    let sup = PgSupervisor::start(data_dir.path().to_path_buf(), "postgres")
        .await
        .expect("start shared embedded PG");
    SharedPg {
        backend: Backend::Embedded {
            sup,
            _data_dir: data_dir,
        },
        db_counter: AtomicU64::new(0),
    }
}

async fn init_external(url: &str) -> SharedPg {
    let admin_opts: PgConnectOptions = url.parse().expect("TERAMIND_TEST_PG_URL parse");
    // Best-effort cleanup of leftover databases from prior runs. Failures here
    // are non-fatal — fresh runs may have nothing to clean.
    if let Ok(admin_pool) = DbPool::connect(admin_opts.clone()).await {
        let names: Vec<(String,)> = sqlx::query_as(
            "SELECT datname FROM pg_database WHERE datname LIKE 'teramind_test_db_%'",
        )
        .fetch_all(admin_pool.pg())
        .await
        .unwrap_or_default();
        for (db,) in names {
            // Ignore drop failures (database may be in use by a parallel
            // run, or already-removed).
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{db}\""))
                .execute(admin_pool.pg())
                .await;
        }
    }
    SharedPg {
        backend: Backend::External { admin_opts },
        db_counter: AtomicU64::new(0),
    }
}

/// Freshly-migrated DbPool in an isolated database within the shared PG.
///
/// Database name is `teramind_test_db_<pid>_<counter>` so parallel test
/// binaries against the same external Postgres don't collide.
pub async fn fresh_pool() -> anyhow::Result<DbPool> {
    let pg = shared().await;
    let n = pg.db_counter.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let db_name = format!("teramind_test_db_{pid}_{n}");

    let admin_pool = DbPool::connect(pg.admin_opts()).await?;
    sqlx::query(&format!("CREATE DATABASE \"{db_name}\""))
        .execute(admin_pool.pg())
        .await?;
    drop(admin_pool);

    let pool = DbPool::connect(pg.db_opts(&db_name)).await?;
    migrate::run(&pool).await?;
    Ok(pool)
}
