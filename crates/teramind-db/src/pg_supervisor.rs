//! Embedded Postgres lifecycle supervisor.
//!
//! Wraps `postgresql_embedded::PostgreSQL`, providing a small, stable façade
//! the rest of the crate (and daemon) depends on. External contract:
//! `start`, `connect_options`, `shutdown`, `data_dir`.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use postgresql_embedded::{PostgreSQL, Settings};
use sqlx::postgres::PgConnectOptions;

use crate::error::{DbError, Result};

/// A running embedded Postgres instance with a single application database.
pub struct PgSupervisor {
    instance: PostgreSQL,
    database_name: String,
    data_dir: PathBuf,
}

impl PgSupervisor {
    /// Install (if needed), initialise (if needed), and start an embedded
    /// Postgres rooted at `data_dir`. Ensures `database_name` exists.
    pub async fn start(data_dir: PathBuf, database_name: &str) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;

        let settings = Settings {
            data_dir: data_dir.clone(),
            password: "teramind".to_string(),
            // Pin to PostgreSQL 16: portal-corp's pgvector_compiled prebuilts
            // only exist for PG16. Update when PG17/18 builds appear.
            version: postgresql_embedded::V16.clone(),
            // The instance owns its data dir; we manage the lifecycle here, so
            // disable the library's auto-cleanup-on-drop behaviour.
            temporary: false,
            ..Settings::default()
        };

        let mut instance = PostgreSQL::new(settings);
        instance
            .setup()
            .await
            .map_err(|e| DbError::Supervisor(format!("setup: {e}")))?;
        instance
            .start()
            .await
            .map_err(|e| DbError::Supervisor(format!("start: {e}")))?;

        // Install pgvector into the embedded PG bundle before any migrations
        // so that `CREATE EXTENSION vector` works inside migrations.
        install_pgvector(instance.settings())
            .await
            .context("install pgvector into embedded PG")
            .map_err(|e| DbError::Supervisor(e.to_string()))?;

        let exists = instance
            .database_exists(database_name)
            .await
            .map_err(|e| DbError::Supervisor(format!("database_exists: {e}")))?;
        if !exists {
            instance
                .create_database(database_name)
                .await
                .map_err(|e| DbError::Supervisor(format!("create_database: {e}")))?;
        }

        Ok(Self {
            instance,
            database_name: database_name.to_string(),
            data_dir,
        })
    }

    /// `PgConnectOptions` for the application database on this instance.
    pub fn connect_options(&self) -> PgConnectOptions {
        let s = self.instance.settings();
        PgConnectOptions::new()
            .host(&s.host)
            .port(s.port)
            .username(&s.username)
            .password(&s.password)
            .database(&self.database_name)
    }

    /// Data directory backing this instance.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Gracefully stop the embedded server.
    pub async fn shutdown(self) -> Result<()> {
        self.instance
            .stop()
            .await
            .map_err(|e| DbError::Supervisor(format!("stop: {e}")))?;
        Ok(())
    }
}

/// Install pgvector into the embedded PostgreSQL bundle.
///
/// Uses the `portal-corp` repository which distributes precompiled pgvector
/// binaries. The extension name in that registry is `pgvector_compiled`.
/// We request any compatible version (`*`) so the latest build for the
/// running PG major version is selected automatically.
///
/// The install is idempotent: if the files already exist the function
/// uninstalls them first (registry bookkeeping) before reinstalling,
/// so repeated `PgSupervisor::start` calls work correctly.
async fn install_pgvector(settings: &Settings) -> anyhow::Result<()> {
    let version = postgresql_extensions::VersionReq::STAR;
    postgresql_extensions::install(settings, "portal-corp", "pgvector_compiled", &version)
        .await
        .context("postgresql_extensions::install portal-corp/pgvector_compiled")?;
    Ok(())
}
