//! Library half of `teramind-search-eval`. The CLI binary in `main.rs`
//! is a thin shell around these modules so they remain testable.
pub mod metrics;
pub mod types;
pub mod corpus;
pub mod generator;
pub mod queries_bank;
pub mod harness;
pub mod reporter;
pub mod gates;
pub mod semantic;
