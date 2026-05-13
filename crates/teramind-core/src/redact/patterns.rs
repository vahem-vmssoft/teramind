// Patterns are added one task at a time as failing tests drive them.

pub struct Pattern {
    pub name: &'static str,
    pub regex: &'static str,
}

pub const PATTERNS: &[Pattern] = &[
    Pattern { name: "aws_access_key", regex: r"AKIA[0-9A-Z]{16}" },
    Pattern { name: "github_token",   regex: r"gh[pousr]_[A-Za-z0-9]{36}" },
];
