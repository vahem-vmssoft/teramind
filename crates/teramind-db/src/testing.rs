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
use std::sync::{Mutex, OnceLock};
use tokio::sync::OnceCell;

static PG: OnceCell<SharedPg> = OnceCell::const_new();

/// Every db name that `fresh_pool` has created in this process. The first
/// push registers an `atexit` hook that drops them all — so a `cargo test`
/// run does not leave behind hundreds of `teramind_test_db_<pid>_<n>`
/// databases on an external Postgres.
static CREATED_DBS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

/// Snapshot of the admin connect options for the exit-time cleanup. We can't
/// reach `PG` through `tokio::sync::OnceCell::get` from inside `atexit`
/// reliably (the test tokio runtime is gone by then), so this is the
/// dedicated escape hatch.
static EXIT_ADMIN_OPTS: OnceLock<PgConnectOptions> = OnceLock::new();

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
    // Stash admin opts for the exit-time cleanup. Embedded mode tears down
    // the whole PG instance with the tempdir, so the cleanup is technically
    // redundant — but it still works (and matches external semantics).
    let _ = EXIT_ADMIN_OPTS.set(sup.connect_options().database("postgres"));
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
    let _ = EXIT_ADMIN_OPTS.set(admin_opts.clone());
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

    let admin_pool = connect_with_retry(pg.admin_opts()).await?;
    sqlx::query(&format!("CREATE DATABASE \"{db_name}\""))
        .execute(admin_pool.pg())
        .await?;
    drop(admin_pool);
    track_for_exit_cleanup(db_name.clone());

    let pool = connect_with_retry(pg.db_opts(&db_name)).await?;
    migrate::run(&pool).await?;
    Ok(pool)
}

/// Register a db name with the exit-time cleanup registry, installing the
/// `atexit` hook on first call.
fn track_for_exit_cleanup(db: String) {
    let lock = CREATED_DBS.get_or_init(|| {
        // SAFETY: `libc::atexit` requires the callback to live for the rest
        // of the process. `cleanup_at_exit` is an `extern "C" fn` with
        // static lifetime, so this contract holds.
        unsafe {
            libc::atexit(cleanup_at_exit);
        }
        Mutex::new(Vec::new())
    });
    if let Ok(mut g) = lock.lock() {
        g.push(db);
    }
}

/// Called via `libc::atexit` after `main` returns. Drops every database we
/// created. Runs on a fresh OS thread + catches all panics so a cleanup
/// failure can never abort the test process — by the time we get here,
/// the test runner has already shut down stderr in some configurations and
/// a panic-during-shutdown would otherwise SIGABRT the whole binary.
extern "C" fn cleanup_at_exit() {
    let _ = std::panic::catch_unwind(|| {
        let Some(reg) = CREATED_DBS.get() else { return };
        let names: Vec<String> = match reg.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(_) => return,
        };
        if names.is_empty() {
            return;
        }
        let Some(admin_opts) = EXIT_ADMIN_OPTS.get().cloned() else {
            return;
        };
        // Run the async work on a brand-new OS thread with its own tokio
        // runtime. The atexit-calling thread's thread-locals may already
        // be torn down at this point; a fresh thread gives the runtime
        // clean state to work with.
        let handle = std::thread::Builder::new()
            .name("teramind-db-test-cleanup".into())
            .spawn(move || {
                let _ = std::panic::catch_unwind(|| {
                    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    else {
                        return;
                    };
                    rt.block_on(async move {
                        let Ok(pool) = DbPool::connect(admin_opts).await else {
                            return;
                        };
                        for n in &names {
                            let _ = sqlx::query(
                                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                                 WHERE datname = $1 AND pid <> pg_backend_pid()",
                            )
                            .bind(n)
                            .execute(pool.pg())
                            .await;
                            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{n}\""))
                                .execute(pool.pg())
                                .await;
                        }
                    });
                });
            });
        if let Ok(h) = handle {
            let _ = h.join();
        }
    });
}

/// Postgres.app on macOS pops a permission dialog the first time an unrecognized
/// binary connects, rejecting that initial attempt with a fatal XX000 error
/// ("Postgres.app rejected \"trust\" authentication"). The user clicks Allow
/// and subsequent connections succeed. Absorb that one-shot rejection by
/// retrying with backoff.
async fn connect_with_retry(opts: PgConnectOptions) -> anyhow::Result<DbPool> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..5 {
        match DbPool::connect(opts.clone()).await {
            Ok(p) => return Ok(p),
            Err(e) => {
                let msg = e.to_string();
                let transient =
                    msg.contains("Postgres.app rejected") || msg.contains("auth_permission_dialog");
                last_err = Some(e.into());
                if !transient {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt + 1))).await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("connect_with_retry exhausted")))
}
