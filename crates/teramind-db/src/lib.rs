//! Embedded Postgres lifecycle, migrations, and per-entity repositories.

pub mod error;
pub mod migrate;
pub mod pg_supervisor;
pub mod pool;
pub mod repos;

pub mod testing;

pub use error::DbError;
pub use pool::DbPool;
