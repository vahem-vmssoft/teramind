//! Shared types, identifiers, error enum, and redaction rules.

pub mod error;
pub mod ids;
// pub mod redact;  // Task 13+ adds this
pub mod types;

pub use error::Error;
pub use ids::*;
pub use types::*;
