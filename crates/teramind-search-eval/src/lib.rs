//! Library half of `teramind-search-eval`. The CLI binary in `main.rs`
//! is a thin shell around these modules so they remain testable.
pub mod corpus;
pub mod gates;
pub mod generator;
pub mod harness;
pub mod metrics;
pub mod queries_bank;
pub mod reporter;
pub mod semantic;
pub mod types;
