// Patterns are added one task at a time as failing tests drive them.

pub struct Pattern {
    pub name: &'static str,
    pub regex: &'static str,
}

pub const PATTERNS: &[Pattern] = &[
    Pattern { name: "aws_access_key", regex: r"AKIA[0-9A-Z]{16}" },
    Pattern { name: "github_token",   regex: r"gh[pousr]_[A-Za-z0-9]{36}" },
    Pattern { name: "slack_token",    regex: r"xox[bpoa]-[A-Za-z0-9-]{10,}" },
    Pattern { name: "jwt",            regex: r"eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+" },
    Pattern { name: "pem_private_key", regex: r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----" },
    Pattern { name: "password_kv", regex: r"(?i)\b(?:password|pwd)\s*=\s*\S+" },
];
