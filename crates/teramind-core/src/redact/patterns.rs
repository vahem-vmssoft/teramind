// Patterns are added one task at a time as failing tests drive them.

pub struct Pattern {
    pub name: &'static str,
    pub regex: &'static str,
}

pub const PATTERNS: &[Pattern] = &[];
