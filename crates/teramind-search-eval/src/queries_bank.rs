//! Hand-curated query bank. Populated in Section 5.

use crate::types::QueryClass;

pub struct QueryBankEntry {
    pub id: &'static str,
    pub class: QueryClass,
    pub text: &'static str,
    pub triggers: &'static [&'static str],
}

// Placeholder so the crate compiles; full bank arrives in Section 5.
pub const QUERIES: &[QueryBankEntry] = &[];
