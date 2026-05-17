use crate::error::Result;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

#[derive(Clone)]
pub struct DbPool {
    pub(crate) inner: PgPool,
}

impl DbPool {
    pub async fn connect(opts: PgConnectOptions) -> Result<Self> {
        let inner = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect_with(opts)
            .await?;
        Ok(Self { inner })
    }
    pub fn pg(&self) -> &PgPool {
        &self.inner
    }

    pub async fn connect_url(url: &str) -> anyhow::Result<Self> {
        let opts: PgConnectOptions = url.parse()?;
        Self::connect(opts).await.map_err(Into::into)
    }
}
