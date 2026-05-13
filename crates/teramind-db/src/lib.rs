//! Embedded Postgres lifecycle, migrations, and per-entity repositories.

pub mod error;
pub mod pool;

pub use error::DbError;
pub use pool::DbPool;
