use crate::error::Result;
use crate::pool::DbPool;

pub async fn run(pool: &DbPool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool.pg()).await?;
    Ok(())
}
