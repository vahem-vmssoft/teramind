//! Embedded Postgres lifecycle, migrations, and per-entity repositories.

pub mod error;
pub mod pg_supervisor;
pub mod pool;
pub mod repos;

pub use error::DbError;
pub use pool::DbPool;
